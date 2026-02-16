use anchor_lang::prelude::*;
use crate::constants::MAX_ROOTS;

/// Marker PDA keyed by `deposit_hash` that makes `shielded_deposit` idempotent.
#[account]
pub struct DepositMarker {
    /// Has this deposit_hash already been consumed (commitment inserted)?
    pub processed: bool,
    /// PDA bump
    pub bump: u8,
}

impl DepositMarker {
    /// Raw field size (excluding the 8-byte Anchor discriminator)
    pub const SIZE: usize = 1 + 1;
    /// Full account space (including discriminator)
    pub const SPACE: usize = 8 + Self::SIZE;

    /// Mark as processed (idempotent setter).
    #[inline]
    pub fn set_processed(&mut self) {
        self.processed = true;
    }
}

/// On-chain nullifier record with audit trail.
/// Stores merkle root and timing information for permanent verification,
/// eliminating dependency on transaction logs which get pruned.
/// 
/// Privacy: Commitments are NOT stored to preserve transaction privacy.
/// Only merkle roots (which don't reveal individual commitments) are stored.
#[account]
pub struct NullifierRecord {
    /// Whether this nullifier has been consumed (spent).
    pub used: bool,

    /// PDA bump.
    pub bump: u8,

    /// Slot when this nullifier was first marked spent.
    pub spent_slot: u64,

    /// Unix timestamp (seconds) when this nullifier was marked spent.
    /// Sourced from the Clock sysvar.
    pub spent_unix_ts: i64,

    /// === Audit Trail Fields (for permanent on-chain verification) ===
    
    /// Merkle root that was proven in the ZK proof (before transaction)
    /// This is the "old root" from the circuit - proves the spent note existed
    pub merkle_root_before: [u8; 32],
    
    /// Final merkle root after all insertions (after transaction)
    /// For transfer: root after both out1 and out2 insertions
    /// For withdraw: same as merkle_root_before (no new commitments)
    pub merkle_root_after: [u8; 32],
    
    /// Event type: 0=unknown, 1=transfer, 2=withdraw
    /// Helps auditors distinguish between transaction types
    pub event_type: u8,
}

impl NullifierRecord {
    // Field sizes:
    // Basic: bool:1 + u8:1 + u64:8 + i64:8 = 18 bytes
    // Audit trail: [u8;32]*2 (two roots) + u8 (event_type) = 65 bytes
    // Total: 18 + 65 = 83 bytes
    pub const SIZE: usize = 1 + 1 + 8 + 8 + (32 * 2) + 1;

    // Anchor discriminator (8) + fields = 91 bytes total
    pub const SPACE: usize = 8 + Self::SIZE;
}


#[account]
pub struct TreeState {
    pub version:     u16,        // v1
    pub current_root:[u8; 32],
    pub next_index:  u32,
    pub depth:       u8,
    pub _reserved:   [u8; 31],   // future flags/fields (optional)
}
// Anchor 0.29+: implement `Space` with `INIT_SPACE`
impl anchor_lang::Space for TreeState {
    const INIT_SPACE: usize = 2 + 32 + 4 + 1 + 31;
}

/// Fixed-capacity ring buffer for recent Merkle roots.
///
/// • Zero-copy: no (de)serialization of a large Vec on every ix.
/// • Backed by a PDA and accessed via `AccountLoader<MerkleRootCache>`.
///
/// Layout on-chain:
///   [8-byte discriminator] + [[u8;32]; MAX_ROOTS] + u16(next_slot) + u16(count)
#[account(zero_copy)]
#[repr(C)]
pub struct MerkleRootCache {
    /// Ring buffer of recent roots.
    pub roots: [[u8; 32]; MAX_ROOTS],
    /// Next write position in the ring (0..MAX_ROOTS-1).
    pub next_slot: u16,
    /// Number of valid entries (<= MAX_ROOTS).
    pub count: u16,
}

impl MerkleRootCache {
    /// Bytes excluding the discriminator.
    pub const BYTE_SIZE: usize = (MAX_ROOTS * 32) + 2 + 2;
    /// Bytes including the discriminator (what you pass as `space` minus the 8 you add in `#[account(init, space = 8 + ...)]`).
    pub const SIZE: usize = Self::BYTE_SIZE;
    /// Convenience: full account size including discriminator.
    pub const SPACE: usize = 8 + Self::BYTE_SIZE;

    #[inline]
    pub fn clear(&mut self) {
        // All zeros is a valid empty state, but we explicitly reset counters.
        self.next_slot = 0;
        self.count = 0;
        // Zero the roots array.
        // (Compiler is smart enough; this does NOT copy on stack.)
        self.roots = [[0u8; 32]; MAX_ROOTS];
    }

    /// Insert a new root (ring-buffer). Overwrites oldest when full.
    #[inline]
    pub fn insert(&mut self, new_root: [u8; 32]) {
        let idx = (self.next_slot as usize) % MAX_ROOTS;
        self.roots[idx] = new_root;
        self.next_slot = ((self.next_slot as usize + 1) % MAX_ROOTS) as u16;
        if (self.count as usize) < MAX_ROOTS {
            self.count += 1;
        }
    }

    /// Check whether a root exists in the cache (O(MAX_ROOTS)).
    #[inline]
    pub fn contains(&self, root: &[u8; 32]) -> bool {
        let total = self.count as usize;
        if total == 0 {
            return false;
        }
        // If not yet full, the logical order is 0..count-1.
        // If full, the oldest is at next_slot.
        let start = if total < MAX_ROOTS {
            0usize
        } else {
            self.next_slot as usize
        };
        for i in 0..total {
            let idx = (start + i) % MAX_ROOTS;
            if &self.roots[idx] == root {
                return true;
            }
        }
        false
    }

    /// Latest (most recently inserted) root, if any.
    #[inline]
    pub fn latest(&self) -> Option<[u8; 32]> {
        if self.count == 0 {
            None
        } else {
            let idx = (self.next_slot as usize + MAX_ROOTS - 1) % MAX_ROOTS;
            Some(self.roots[idx])
        }
    }
}
