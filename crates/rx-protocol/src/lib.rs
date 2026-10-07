//! Generated v1 wire types and strict decoding. Domain logic must not depend on this crate.
pub mod checkpoint_rejection;
pub mod json;
pub mod strict;

pub mod rx {
    pub mod resident {
        pub mod execution {
            pub mod v1 {
                tonic::include_proto!("rx.resident.execution.v1");
            }
        }
        pub mod reporting {
            pub mod v1 {
                tonic::include_proto!("rx.resident.reporting.v1");
            }
        }
    }
    pub mod host {
        pub mod execution {
            pub mod v2 {
                tonic::include_proto!("rx.host.execution.v2");
            }
        }
        pub mod qualification {
            pub mod v1 {
                tonic::include_proto!("rx.host.qualification.v1");
            }
            pub mod v2 {
                tonic::include_proto!("rx.host.qualification.v2");
            }
        }
        pub mod configuration {
            pub mod v2 {
                tonic::include_proto!("rx.host.configuration.v2");
            }
            pub mod v1 {
                tonic::include_proto!("rx.host.configuration.v1");
            }
        }
        pub mod read {
            pub mod v1 {
                tonic::include_proto!("rx.host.read.v1");
            }
        }
    }
    pub mod executor {
        pub mod execution {
            pub mod v2 {
                tonic::include_proto!("rx.executor.execution.v2");
            }
        }
        pub mod assignment {
            pub mod v1 {
                tonic::include_proto!("rx.executor.assignment.v1");
            }
        }
        pub mod production {
            pub mod v1 {
                tonic::include_proto!("rx.executor.production.v1");
            }
        }
        pub mod plan {
            pub mod v1 {
                tonic::include_proto!("rx.executor.plan.v1");
            }
        }
        pub mod v1 {
            tonic::include_proto!("rx.executor.v1");
        }
    }
    pub mod contract {
        pub mod v1 {
            tonic::include_proto!("rx.contract.v1");
        }
    }
    pub mod cell {
        pub mod v1 {
            tonic::include_proto!("rx.cell.v1");
        }
    }
}
pub use rx::cell::v1 as cell;
pub use rx::contract::v1 as base;
pub use rx::executor::plan::v1 as executor_plan;
pub use rx::executor::v1 as executor;

pub const DESCRIPTOR_BYTES: &[u8] = tonic::include_file_descriptor_set!("rx_descriptor");
pub static DESCRIPTORS: std::sync::LazyLock<prost_reflect::DescriptorPool> =
    std::sync::LazyLock::new(|| {
        prost_reflect::DescriptorPool::decode(DESCRIPTOR_BYTES)
            .expect("build-generated descriptor must be valid")
    });

pub use rx::executor::production::v1 as production;

pub use rx::host::read::v1 as host_read;

pub use rx::host::configuration::v1 as host_configuration;

pub use rx::host::qualification::v1 as host_qualification;

pub use rx::executor::assignment::v1 as assignment;
pub use rx::resident::reporting::v1 as resident_reporting;

pub use rx::resident::execution::v1 as resident_execution;

pub use rx::host::configuration::v2 as host_execution_configuration;

pub use rx::host::qualification::v2 as host_execution_qualification;

pub use rx::executor::execution::v2 as execution_v2;

pub use rx::host::execution::v2 as host_execution;
