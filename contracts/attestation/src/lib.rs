#![no_std]
use core::cmp::Ordering;
use soroban_sdk::{contract, contractimpl, Address, BytesN, Env, String, Vec};

const STATUS_KEY_TAG: u32 = 1;
const ADMIN_KEY_TAG: (u32,) = (2,);
const QUERY_LIMIT_MAX: u32 = 30;

pub const STATUS_ACTIVE: u32 = 0;
pub const STATUS_REVOKED: u32 = 1;
pub const STATUS_FILTER_ALL: u32 = 2;

#[contract]
pub struct AttestationContract;

#[contractimpl]
impl AttestationContract {
    /// Submit a revenue attestation: store merkle root and metadata for (business, period).
    /// Prevents overwriting existing attestation for the same period (idempotency).
    /// New attestations are stored with status active (0).
    pub fn submit_attestation(
        env: Env,
        business: Address,
        period: String,
        merkle_root: BytesN<32>,
        timestamp: u64,
        version: u32,
    ) {
        let key = (business.clone(), period.clone());
        if env.storage().instance().has(&key) {
            panic!("attestation already exists for this business and period");
        }
        let data = (merkle_root, timestamp, version);
        env.storage().instance().set(&key, &data);
        let status_key = (STATUS_KEY_TAG, business, period);
        env.storage().instance().set(&status_key, &STATUS_ACTIVE);
    }

    /// Return stored attestation for (business, period) if any.
    pub fn get_attestation(
        env: Env,
        business: Address,
        period: String,
    ) -> Option<(BytesN<32>, u64, u32)> {
        let key = (business, period);
        env.storage().instance().get(&key)
    }

    /// Verify that an attestation exists and matches the given merkle root.
    pub fn verify_attestation(
        env: Env,
        business: Address,
        period: String,
        merkle_root: BytesN<32>,
    ) -> bool {
        if let Some((stored_root, _ts, _ver)) = Self::get_attestation(env.clone(), business, period)
        {
            stored_root == merkle_root
        } else {
            false
        }
    }

    /// One-time setup of admin. Admin is the only address that may revoke attestations.
    pub fn init(env: Env, admin: Address) {
        admin.require_auth();
        if env.storage().instance().has(&ADMIN_KEY_TAG) {
            panic!("admin already set");
        }
        env.storage().instance().set(&ADMIN_KEY_TAG, &admin);
    }

    /// Revoke an attestation. Caller must be admin. Status is set to revoked (1).
    pub fn revoke_attestation(env: Env, caller: Address, business: Address, period: String) {
        caller.require_auth();
        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY_TAG)
            .expect("admin not set");
        if caller != admin {
            panic!("caller is not admin");
        }
        let attest_key = (business.clone(), period.clone());
        if !env.storage().instance().has(&attest_key) {
            panic!("attestation does not exist");
        }
        let status_key = (STATUS_KEY_TAG, business, period);
        env.storage().instance().set(&status_key, &STATUS_REVOKED);
    }

    /// Returns status for (business, period): 0 active, 1 revoked. Defaults to active if not set.
    fn get_status(env: &Env, business: &Address, period: &String) -> u32 {
        let key = (STATUS_KEY_TAG, business.clone(), period.clone());
        env.storage()
            .instance()
            .get(&key)
            .unwrap_or(STATUS_ACTIVE)
    }

    /// Paginated query: returns attestations for the given business and period list, with optional filters.
    /// periods: list of period strings to consider (e.g. from an indexer). Cursor indexes into this list.
    /// period_start: include only period >= this (None = no lower bound). period_end: include only period <= this (None = no upper bound).
    /// status_filter: 0 active only, 1 revoked only, 2 all. version_filter: None = any version.
    /// limit: max results (capped at QUERY_LIMIT_MAX). cursor: index into periods to start from.
    /// Returns (results as Vec of (period, merkle_root, timestamp, version, status), next_cursor).
    /// Next_cursor is cursor + number of periods scanned (not result count). DoS-limited by cap on limit and bounded reads.
    pub fn get_attestations_page(
        env: Env,
        business: Address,
        periods: Vec<String>,
        period_start: Option<String>,
        period_end: Option<String>,
        status_filter: u32,
        version_filter: Option<u32>,
        limit: u32,
        cursor: u32,
    ) -> (Vec<(String, BytesN<32>, u64, u32, u32)>, u32) {
        let limit = core::cmp::min(limit, QUERY_LIMIT_MAX);
        let len = periods.len();
        if cursor >= len {
            return (Vec::new(&env), cursor);
        }
        let mut out = Vec::new(&env);
        let mut scanned: u32 = 0;
        let mut i = cursor;
        while i < len && (out.len() as u32) < limit {
            let period = periods.get(i).unwrap();
            let in_range = period_start
                .as_ref()
                .map_or(true, |s| period.cmp(s) != Ordering::Less)
                && period_end
                    .as_ref()
                    .map_or(true, |s| period.cmp(s) != Ordering::Greater);
            if !in_range {
                i += 1;
                scanned += 1;
                continue;
            }
            let key = (business.clone(), period.clone());
            if let Some((root, ts, ver)) = env.storage().instance().get::<_, (BytesN<32>, u64, u32)>(&key) {
                let status = Self::get_status(&env, &business, &period);
                let status_ok = status_filter == STATUS_FILTER_ALL
                    || (status_filter == STATUS_ACTIVE && status == STATUS_ACTIVE)
                    || (status_filter == STATUS_REVOKED && status == STATUS_REVOKED);
                let version_ok =
                    version_filter.map_or(true, |v| v == ver);
                if status_ok && version_ok {
                    out.push_back((period.clone(), root, ts, ver, status));
                }
            }
            i += 1;
            scanned += 1;
        }
        (out, cursor + scanned)
    }
}

mod test;
#[cfg(test)]
mod query_pagination_test;
