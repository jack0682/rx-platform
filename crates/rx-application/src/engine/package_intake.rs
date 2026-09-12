use super::*;
use crate::package_intake::*;
const SERVICE: &str = "rx.internal.package-intake-service.v1";
const RECEIPT: &str = "rx.package-intake-receipt.v1";
#[derive(Serialize, serde::Deserialize)]
struct Service {
    boot: Id,
    registration: Option<Registration>,
}
pub(super) fn configuration_digest(cell: &Cell) -> Result<Digest> {
    canonical::digest("RX-PACKAGE-INTAKE-CELL-CONTEXT-v1", &cell.configuration)
        .map_err(domain_error)
}
pub(super) fn current(
    tx: &mut dyn Transaction,
    meta: &Installation,
) -> Result<Option<Registration>> {
    let Some(row) = tx.get(&name("packageintake/service"))? else {
        return Ok(None);
    };
    let value: Service = decode(&row, SERVICE)?;
    Ok(if value.boot == meta.runtime_boot {
        value.registration
    } else {
        None
    })
}
fn view(receipt: Receipt, cell: &Cell, registration: &Option<Registration>) -> Result<View> {
    Ok(View {
        review_context_current: registration.as_ref() == Some(&receipt.registration)
            && configuration_digest(cell)? == receipt.configuration_digest,
        receipt,
        content_reverification_required: true,
        activation_authorized: false,
    })
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// Trusted composition only. Not exposed as a browser/RPC policy registration endpoint.
    pub fn configure_package_intake(
        &mut self,
        configuration: Option<(Id, Digest, Digest)>,
    ) -> Result<Option<Registration>> {
        let meta = &self.installation;
        self.repository.transact(|tx| {
            lifecycle::require_serving(tx)?;
            let k = name("packageintake/service");
            let old = tx.get(&k)?;
            let registration =
                configuration.map(|(store_owner, policy_fingerprint, policy_file_digest)| {
                    Registration {
                        generation: id(),
                        store_owner,
                        policy_fingerprint,
                        policy_file_digest,
                    }
                });
            let value = Service {
                boot: meta.runtime_boot.clone(),
                registration: registration.clone(),
            };
            tx.put(&k, old.map(|r| r.revision), &doc(SERVICE, &value)?)?;
            event(tx, "rx.event.package-intake-service-configured.v1", &value)?;
            qualification_activation::suspend_changed_roots(tx, meta)?;
            Ok(registration)
        })
    }
    pub fn package_intake_context(
        &mut self,
        identity: &Identity,
        cell: &Name,
    ) -> Result<crate::package_intake::Context> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            process_draft::read_access(tx, identity, meta, &clock.now(), cell)?;
            let (_, value): (_, Cell) = load(tx, "cell", cell, CELL)?;
            Ok(crate::package_intake::Context {
                device_review_authority_digest: super::device_review::authority(tx, meta)?,
                review_authority_digest: super::process_review::authority(tx, meta)?,
                cell: cell.clone(),
                configuration_digest: configuration_digest(&value)?,
                registration: current(tx, meta)?,
            })
        })
    }
    pub fn prepare_package_intake(
        &mut self,
        identity: &Identity,
        request_key: &Id,
        input: Submit,
    ) -> Result<Preflight> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let principal = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&input.cell),
                Role::Engineer,
                false,
            )?;
            if input.title.trim().is_empty() || input.title.chars().count() > 120 {
                return reject(Reject::InvalidInput);
            }
            let (_, cell): (_, Cell) = load(tx, "cell", &input.cell, CELL)?;
            // Historical receipt recovery is allowed only after current identity/scope checks.
            // It does not repeat verification or grant approval; GET exposes current context separately.
            let (scope, fp) = request(
                meta,
                &principal,
                "PackageIntake.Submit",
                request_key.as_str(),
                &input,
            )?;
            if let Some(old) = prior(tx, &scope, fp, RECEIPT)? {
                return Ok(Preflight::Recorded(Box::new(old)));
            }
            let registration = current(tx, meta)?.ok_or(StoreError::Unavailable(
                "package intake service is not configured".into(),
            ))?;
            if registration.generation != input.policy_generation
                || configuration_digest(&cell)? != input.configuration_digest
            {
                return reject(Reject::StaleRevision);
            }
            if tx.get(&key("packageintakereceipt", &input.id))?.is_some() {
                return reject(Reject::StaleRevision);
            }
            Ok(Preflight::Verify(Box::new(Ticket {
                input,
                identity: identity.clone(),
                request_key: request_key.clone(),
                boot: meta.runtime_boot.clone(),
                registration,
                expires_at: TimePoint {
                    clock_id: now.clock_id.clone(),
                    ticks_ns: Counter(
                        now.ticks_ns
                            .0
                            .checked_add(30_000_000_000)
                            .ok_or(StoreError::Rejected(Reject::InvalidInput))?,
                    ),
                },
                issued_at: now,
            })))
        })
    }
    pub fn commit_package_intake(&mut self, prepared: Prepared) -> Result<Receipt> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let ticket = &prepared.ticket;
            let command = &ticket.input;
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let principal = authorize(
                tx,
                &ticket.identity,
                meta,
                &now,
                Some(&command.cell),
                Role::Engineer,
                false,
            )?;
            let (scope, fp) = request(
                meta,
                &principal,
                "PackageIntake.Submit",
                ticket.request_key.as_str(),
                command,
            )?;
            if let Some(old) = prior(tx, &scope, fp, RECEIPT)? {
                return Ok(old);
            }
            let (_, cell): (_, Cell) = load(tx, "cell", &command.cell, CELL)?;
            if ticket.boot != meta.runtime_boot
                || current(tx, meta)?.as_ref() != Some(&ticket.registration)
                || configuration_digest(&cell)? != command.configuration_digest
            {
                return reject(Reject::StaleRevision);
            }
            if now.age_ns(&ticket.issued_at).is_none()
                || now.clock_id != ticket.expires_at.clock_id
                || now.ticks_ns >= ticket.expires_at.ticks_ns
            {
                return reject(Reject::StaleRevision);
            }
            if prepared.stored.object() != &command.object
                || prepared.stored.owner() != &ticket.registration.store_owner
                || prepared.stored.policy_fingerprint() != ticket.registration.policy_fingerprint
            {
                return reject(Reject::InvalidInput);
            }
            if tx.get(&key("packageintakereceipt", &command.id))?.is_some() {
                return reject(Reject::StaleRevision);
            }
            let device_catalog = if let Some(c) = &prepared.device_catalog {
                c.validate().map_err(StoreError::Invalid)?;
                let environment = match cell.configuration.environment {
                    Environment::Simulation => {
                        rx_process_contract::device_catalog::Environment::Simulation
                    }
                    Environment::Physical => {
                        rx_process_contract::device_catalog::Environment::Physical
                    }
                };
                if c.installation != meta.id
                    || c.cell != command.cell
                    || c.environment != environment
                {
                    return reject(Reject::InvalidInput);
                }
                let data = canonical::bytes(c).map_err(domain_error)?;
                let reference = ArtifactRef {
                    sha256: rx_package::content_digest(&data),
                    schema_id: name("rx.device-operation-catalog.v1"),
                    size_bytes: Counter(data.len() as u64),
                };
                let key_ = key("devicecatalog", reference.sha256);
                let incoming = doc("rx.device-operation-catalog.v1", c)?;
                if let Some(old) = tx.get(&key_)? {
                    if old.document != incoming {
                        return Err(StoreError::Integrity(
                            "device catalog digest conflict".into(),
                        ));
                    }
                } else {
                    tx.put(&key_, None, &incoming)?;
                }
                Some(reference)
            } else {
                None
            };
            let receipt = Receipt {
                device_catalog,
                id: command.id.clone(),
                cell: command.cell.clone(),
                title: command.title.clone(),
                object: command.object.clone(),
                configuration_digest: command.configuration_digest,
                registration: ticket.registration.clone(),
                manifest: prepared.stored.package().manifest().clone(),
                submitted_by: principal.id,
                terminal: ticket.identity.terminal.as_ref().map(|(id, _)| id.clone()),
                submitted_at: now,
                state: State::AwaitingReview,
            };
            save(
                tx,
                "packageintakereceipt",
                &receipt.id,
                None,
                RECEIPT,
                &receipt,
            )?;
            event(tx, "rx.event.package-intake-submitted.v1", &receipt)?;
            remember(tx, &scope, fp, RECEIPT, &receipt)?;
            Ok(receipt)
        })
    }
    pub fn package_intake(&mut self, identity: &Identity, cell: &Name, id: &Id) -> Result<View> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            process_draft::read_access(tx, identity, meta, &clock.now(), cell)?;
            let (_, receipt): (_, Receipt) = load(tx, "packageintakereceipt", id, RECEIPT)?;
            if &receipt.cell != cell {
                return reject(Reject::Forbidden);
            }
            let (_, cell): (_, Cell) = load(tx, "cell", cell, CELL)?;
            view(receipt, &cell, &current(tx, meta)?)
        })
    }
    pub fn package_device_catalog(
        &mut self,
        identity: &Identity,
        cell: &Name,
        id: &Id,
    ) -> Result<crate::device_catalog::Detail> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            process_draft::read_access(tx, identity, meta, &clock.now(), cell)?;
            let (_, receipt): (_, Receipt) = load(tx, "packageintakereceipt", id, RECEIPT)?;
            if &receipt.cell != cell {
                return reject(Reject::Forbidden);
            }
            let (_, value): (_, Cell) = load(tx, "cell", cell, CELL)?;
            let current = current(tx, meta)?;
            let catalog = if let Some(reference) = &receipt.device_catalog {
                let (_, c): (_, rx_process_contract::device_catalog::Catalog) = load(
                    tx,
                    "devicecatalog",
                    reference.sha256,
                    "rx.device-operation-catalog.v1",
                )?;
                c.validate().map_err(StoreError::Integrity)?;
                let bytes = canonical::bytes(&c).map_err(domain_error)?;
                if reference.schema_id.as_str() != "rx.device-operation-catalog.v1"
                    || rx_package::content_digest(&bytes) != reference.sha256
                    || bytes.len() as u64 != reference.size_bytes.0
                    || c.cell != *cell
                    || c.installation != meta.id
                {
                    return Err(StoreError::Integrity("stored catalog differs".into()));
                }
                Some(c)
            } else {
                None
            };
            let view = view(receipt, &value, &current)?;
            Ok(crate::device_catalog::Detail {
                cell: cell.clone(),
                intake: id.clone(),
                object: view.receipt.object,
                reference: view.receipt.device_catalog,
                catalog,
                review_context_current: view.review_context_current,
                content_reverification_required: true,
                manufacturer_validation_required: true,
                activation_authorized: false,
            })
        })
    }
    pub fn package_intakes(
        &mut self,
        identity: &Identity,
        cell: &Name,
        after: Option<&Id>,
    ) -> Result<Page> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            process_draft::read_access(tx, identity, meta, &clock.now(), cell)?;
            let (_, value): (_, Cell) = load(tx, "cell", cell, CELL)?;
            let registration = current(tx, meta)?;
            let mut receipts = tx
                .scan("packageintakereceipt/")?
                .iter()
                .map(|r| decode::<Receipt>(r, RECEIPT))
                .collect::<Result<Vec<_>>>()?;
            receipts.retain(|r| &r.cell == cell && after.is_none_or(|id| &r.id > id));
            receipts.sort_by(|a, b| a.id.cmp(&b.id));
            let more = receipts.len() > 50;
            receipts.truncate(50);
            let next = if more {
                receipts.last().map(|r| r.id.clone())
            } else {
                None
            };
            Ok(Page {
                cell: cell.clone(),
                packages: receipts
                    .into_iter()
                    .map(|r| view(r, &value, &registration))
                    .collect::<Result<_>>()?,
                next,
            })
        })
    }
}
