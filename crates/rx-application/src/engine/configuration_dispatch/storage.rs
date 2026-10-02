use super::*;
#[cfg(test)]
#[allow(dead_code)]
#[path = "../../../../rx-process-contract/tests/support/execution_v2_fixture.rs"]
mod fixture;
const V2: &str = "rx.internal.host-configuration-task.v2";
const MAX: usize = 8 * 1024 * 1024;
const BLOBS: crate::artifact_storage::BlobStore =
    crate::artifact_storage::BlobStore::new("hostexecutiontask", MAX as u64);
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    id: Id,
    artifact: ArtifactRef,
}
pub(super) fn decode_task(tx: &mut dyn Transaction, row: &rx_ports::Record) -> Result<Task> {
    if row.document.schema.as_str() == TASK {
        let t: Task = decode(row, TASK)?;
        if !t.execution_policies.is_empty() {
            return Err(StoreError::Integrity("v2 task in legacy storage".into()));
        }
        return Ok(t);
    }
    let stored: Stored = decode(row, V2)?;
    let bytes = BLOBS.read(tx, &stored.artifact)?;
    let task: Task =
        serde_json::from_slice(&bytes).map_err(|e| StoreError::Integrity(e.to_string()))?;
    if task.id != stored.id
        || stored.artifact.schema_id.as_str() != V2
        || task.execution_policies.is_empty()
        || canonical::bytes(&task).map_err(domain_error)? != bytes
    {
        return Err(StoreError::Integrity(
            "v2 task storage identity/canonical bytes".into(),
        ));
    }
    Ok(task)
}
pub(super) fn persist(tx: &mut dyn Transaction, t: &Task, expected: Option<Counter>) -> Result<()> {
    if t.execution_policies.is_empty() {
        save(tx, "hostconfigtask", &t.id, expected, TASK, t)?;
        return event(tx, "rx.event.host-configuration-task.v1", t);
    }
    tx.require_workflow_execution_reader()?;
    let bytes = canonical::bytes(t).map_err(domain_error)?;
    if bytes.len() > MAX {
        return reject(Reject::InvalidInput);
    }
    let artifact = ArtifactRef {
        schema_id: name(V2),
        sha256: rx_package::content_digest(&bytes),
        size_bytes: Counter(bytes.len() as u64),
    };
    BLOBS.put(tx, artifact.sha256, &bytes)?;
    let stored = Stored {
        id: t.id.clone(),
        artifact,
    };
    save(tx, "hostconfigtask", &t.id, expected, V2, &stored)?;
    event(tx, "rx.event.host-configuration-task.v2", &stored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rx_ports::Repository;
    #[test]
    fn v2_task_original_request_survives_chunked_storage_and_restart() {
        let observation = fixture::host_v2_observation();
        let receipt = observation.receipt.clone().unwrap();
        let request = receipt.request.clone();
        let context = &request.context;
        let mut task = Task {
            execution_policies: request.policies.clone(),
            id: context.id.clone(),
            change: context.change.clone(),
            origin: context.cells[0].cell.clone(),
            preparation: context.preparation,
            plan_digest: context.plan_digest,
            host: context.host.clone(),
            host_boot: context.expected_host_boot.clone(),
            delivery_journal: context.expected_delivery_journal.clone(),
            producer_session: fixture::id(21),
            runtime_boot: fixture::id(22),
            cells: context
                .cells
                .iter()
                .map(|c| CellProjection {
                    cell: c.cell.clone(),
                    definition: c.definition,
                    envelope: c.envelope,
                    environment: c.environment.clone(),
                    before_configuration: c.before_configuration,
                    after_configuration: c.after_configuration,
                    recipe: c.recipe.clone(),
                    required_intents: c.required_intents.clone(),
                    required_conditions: c.required_conditions.clone(),
                    epoch: c.epoch,
                    scopes: c.scopes.clone(),
                    fence_request: c.fence_request.clone(),
                    change_blocks: vec![],
                })
                .collect(),
            phase: Phase::SendEntered,
            request_digest: Some(request.digest().unwrap()),
            request: Some(request.into()),
            receipt: Some(Receipt::V2(Box::new(receipt))),
            observation: Some(observation.into()),
            issue: None,
            integrity_disputed: false,
            sender: Sender {
                principal: fixture::n("release"),
                session: fixture::id(23),
                terminal: fixture::n("terminal"),
                certificate: fixture::digest(24),
            },
            created_at: TimePoint {
                clock_id: "storage-only-size-probe".repeat(50000),
                ticks_ns: Counter(1),
            },
        };
        // Deliberately oversized audit metadata probes storage, not clock/admission validity.
        let bytes = canonical::bytes(&task).unwrap();
        assert!(bytes.len() > canonical::MAX_MESSAGE_BYTES);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.db");
        let mut repo = rx_storage::SqliteRepository::open(&path).unwrap();
        repo.transact(|tx| persist(tx, &task, None)).unwrap();
        drop(repo);
        let mut repo = rx_storage::SqliteRepository::open(&path).unwrap();
        repo.transact(|tx| {
            let (revision, loaded) = super::super::read(tx, &task.id)?;
            assert_eq!(revision, Counter(1));
            assert_eq!(canonical::bytes(&loaded).unwrap(), bytes);
            assert_eq!(
                loaded.request.as_ref().unwrap().digest().unwrap(),
                task.request_digest.unwrap()
            );
            Ok(())
        })
        .unwrap();
        task.observation = None;
        task.receipt = None;
        task.issue = Some(Issue::TransportUnavailable);
        repo.transact(|tx| persist(tx, &task, Some(Counter(1))))
            .unwrap();
        repo.transact(|tx| {
            let (_, loaded) = super::super::read(tx, &task.id)?;
            assert_eq!(loaded.phase, Phase::SendEntered);
            assert!(loaded.receipt.is_none());
            assert!(loaded.request.as_ref().unwrap().is_v2());
            Ok(())
        })
        .unwrap();
        assert!(
            repo.snapshot()
                .unwrap()
                .1
                .iter()
                .all(|r| canonical::bytes(&r.document).unwrap().len()
                    <= canonical::MAX_MESSAGE_BYTES)
        );
        // Exercise the actual writer ingress on this stored exchange, without claiming
        // an approved change/native operating path from the storage fixture.
        struct Clock;
        impl crate::Clock for Clock {
            fn now(&self) -> TimePoint {
                TimePoint {
                    clock_id: "test".into(),
                    ticks_ns: Counter(1000),
                }
            }
        }
        struct Deny;
        impl crate::QualificationAuthority for Deny {
            fn verify(
                &self,
                _: &crate::CellConfiguration,
                _: &[ArtifactRef],
                _: &[Digest],
            ) -> bool {
                false
            }
        }
        let principal = |who: &str, roles: Vec<crate::Role>| crate::Principal {
            id: fixture::n(who),
            client_namespace: fixture::n(who),
            roles: roles.into_iter().collect(),
            cells: [fixture::n("cell")].into(),
            active: true,
        };
        let mut app = crate::Engine::open(
            repo,
            Clock,
            Deny,
            fixture::id(31),
            principal("admin", vec![crate::Role::AccountAdmin]),
        )
        .unwrap();
        let expires = TimePoint {
            clock_id: "test".into(),
            ticks_ns: Counter(100000),
        };
        let admin_session = app
            .authenticated_session(&fixture::n("admin"), fixture::id(32), expires.clone())
            .unwrap();
        let admin = crate::Identity {
            principal: fixture::n("admin"),
            session: admin_session.id,
            terminal: None,
        };
        app.put_principal(&admin, principal("host", vec![crate::Role::Host]), None)
            .unwrap();
        let session = app
            .authenticated_session(&fixture::n("host"), fixture::id(33), expires)
            .unwrap();
        let host = crate::Identity {
            principal: fixture::n("host"),
            session: session.id,
            terminal: None,
        };
        let observation = fixture::host_v2_observation();
        let legacy = rx_domain::host_configuration::Observation {
            schema: fixture::n("rx.host-process-configuration-observation.v1"),
            snapshot: observation.snapshot.clone(),
            receipt: Some(observation.receipt.as_ref().unwrap().context.clone()),
            context_matches_current_host: true,
            activation_authorized: false,
        };
        assert!(matches!(
            app.record_host_configuration_observation(&host, &task.id, legacy),
            Err(StoreError::Rejected(Reject::UnsupportedSchema))
        ));
        let confirmed = app
            .record_host_configuration_observation(&host, &task.id, observation.clone())
            .unwrap();
        assert!(confirmed.receipt.as_ref().unwrap().is_v2());
        assert!(!confirmed.integrity_disputed);
        let mut conflicting = observation;
        let receipt = conflicting.receipt.as_mut().unwrap();
        receipt.context.sequence = Counter(2);
        receipt
            .policies
            .values_mut()
            .for_each(|p| p.receipt_sequence = Counter(2));
        conflicting
            .policies
            .values_mut()
            .for_each(|p| p.receipt_sequence = Counter(2));
        conflicting
            .snapshot
            .cells
            .iter_mut()
            .for_each(|c| c.applied.as_mut().unwrap().receipt_sequence = Counter(2));
        conflicting.validate().unwrap();
        let disputed = app
            .record_host_configuration_observation(&host, &task.id, conflicting)
            .unwrap();
        assert!(disputed.integrity_disputed);
        assert_eq!(disputed.receipt.unwrap().context().sequence, Counter(1));
    }
}
