//! Focused invariant scenario tests (see docs/invariant-traceability.json).
use super::*;
use rx_application::{
    diagnostics::{ConditionReason, SourceIssue},
    observation::{Disposition, HostRead},
};
use rx_domain::condition::Verdict;

fn linked() -> (Fixture, rx_application::host_link::Prepare, Id) {
    let (mut f, input) = link_fixture();
    let plan = f.app.prepare_host_link(input.clone()).unwrap();
    f.app.commit_host_link(link_commit(&f, &plan)).unwrap();
    (f, input, plan.id)
}

fn start_attempt(f: &mut Fixture) -> rx_ports::Result<StartAttempt> {
    let revision = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .0;
    let run = f
        .app
        .create_run(&f.operator, id().as_str(), create_command(f, revision))
        .unwrap();
    f.app.start_run(
        &f.operator,
        id().as_str(),
        start_command(f, &run, revision, 1),
    )
}

/// I09, time clause, on the pinned Host-read ingestion path: delivery delay, a time base P cannot
/// order, a capture in P's future and uncertainty beyond the source contract are never hidden to
/// present a sample as fresh.
#[test]
fn host_read_delay_unorderable_time_and_excess_uncertainty_never_present_a_fresh_sample() {
    // Freshness is measured from acquisition, not from arrival at P.
    for (arrival, fresh) in [(21_000, true), (21_001, false)] {
        let (mut f, input, plan) = linked();
        let acquired = input.snapshot.observations[0].acquired_at.clone();
        let maximum_age = f.configuration.fact_specs[0].maximum_age_ns;
        assert_eq!(acquired.ticks_ns.0 + maximum_age.0, 21_000);
        f.clock.0.store(arrival, Ordering::SeqCst);
        let receipt = f
            .app
            .ingest_host_read(HostRead {
                plan,
                snapshot: input.snapshot,
                read_started: input.read_started,
            })
            .unwrap();
        assert_eq!(receipt.entries[0].disposition, Disposition::Current);
        let fact = f
            .app
            .inspect_fact(&f.operator, &f.configuration.id, &name("ready"))
            .unwrap();
        assert_eq!(fact.acquired_at, acquired);
        let view = f.app.overview(&f.operator).unwrap();
        let d = &view.cells[0].diagnostics;
        assert_eq!(
            d.sources[0].age_ns,
            Some(Counter(arrival - acquired.ticks_ns.0))
        );
        assert_eq!(d.sources[0].usable, fresh);
        assert_eq!(
            d.sources[0].issues.contains(&SourceIssue::AgeExceeded),
            !fresh
        );
        if fresh {
            assert_eq!(d.conditions[0].verdict, Verdict::Pass);
            assert!(start_attempt(&mut f).is_ok());
        } else {
            assert_eq!(d.conditions[0].verdict, Verdict::Unknown);
            assert!(matches!(
                d.conditions[0].reason,
                ConditionReason::SourceUnavailable
            ));
            assert!(matches!(
                start_attempt(&mut f),
                Err(StoreError::Rejected(Rejection::ConditionUnknown))
            ));
        }
    }
    // A time base P cannot order, a capture after P's now and excess uncertainty write nothing.
    for mutation in ["foreign-clock", "future-capture", "excess-uncertainty"] {
        let (mut f, input, plan) = linked();
        let mut snapshot = input.snapshot;
        match mutation {
            "foreign-clock" => {
                snapshot.captured_at.clock_id = "other-boot/boottime".into();
                snapshot.observations[0].acquired_at.clock_id = "other-boot/boottime".into();
            }
            "future-capture" => {
                let later = Counter(f.clock.now().ticks_ns.0 + 1);
                snapshot.captured_at.ticks_ns = later;
                snapshot.observations[0].acquired_at.ticks_ns = later;
            }
            "excess-uncertainty" => {
                let limit = f.configuration.fact_specs[0].maximum_uncertainty_ns;
                snapshot.observations[0].uncertainty_ns = Counter(limit.0 + 1);
            }
            _ => unreachable!(),
        }
        let expected = if mutation == "excess-uncertainty" {
            Rejection::InvalidInput
        } else {
            Rejection::ConditionUnknown
        };
        let result = f.app.ingest_host_read(HostRead {
            plan,
            snapshot,
            read_started: input.read_started,
        });
        assert!(
            matches!(result, Err(StoreError::Rejected(r)) if r == expected),
            "{mutation}"
        );
        assert!(
            f.app
                .inspect_fact(&f.operator, &f.configuration.id, &name("ready"))
                .is_err(),
            "{mutation}"
        );
        let view = f.app.overview(&f.operator).unwrap();
        let d = &view.cells[0].diagnostics;
        assert!(
            d.sources[0]
                .issues
                .contains(&SourceIssue::MissingObservation),
            "{mutation}"
        );
        assert!(!d.sources[0].usable, "{mutation}");
        assert_eq!(d.conditions[0].verdict, Verdict::Unknown, "{mutation}");
        assert!(
            matches!(
                start_attempt(&mut f),
                Err(StoreError::Rejected(Rejection::ConditionUnknown))
            ),
            "{mutation}"
        );
    }
}
