//! Rotated development release root. No runtime input can replace these bytes.
//! Development custody is established by the reviewed two-copy ceremony.
//! Product release custody remains NOT_ESTABLISHED.
pub const KEY_ID: &str = "rx/development-release-2026-09-r2";
pub const CHANNEL: &str = "rx/solutions-development";
pub const PUBLIC_KEY: [u8; 32] = [
    0xab, 0x03, 0xa3, 0xf4, 0x96, 0x36, 0x49, 0x92, 0xcb, 0x1f, 0xb9, 0x7d, 0x19, 0x2d, 0x87, 0x49,
    0x46, 0x5e, 0x07, 0xdf, 0x57, 0xec, 0xc9, 0xb3, 0x28, 0x87, 0xf0, 0x17, 0xfd, 0x44, 0xcf, 0x18,
];
pub const DEVELOPMENT_SIGNING_CUSTODY: &str = "ESTABLISHED_BY_TWO_COPY_RECOVERY_REHEARSAL";
pub const PRODUCT_SIGNING_CUSTODY: &str = "NOT_ESTABLISHED";
pub const OFFLINE_REVOCATION_FRESHNESS: &str = "NOT_ESTABLISHED";
pub const PREVIOUS_KEY_ID: &str = "rx/development-release-2026-09";
pub const PREVIOUS_ROOT_STATUS: &str = "RETIRED_NOT_ACCEPTED";
pub const DUAL_ROOT_WINDOW: bool = false;
