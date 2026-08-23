use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::domain::ports::DiceRollerPort;

pub struct Sha256DiceRoller {
    secret: Vec<u8>,
}

impl Sha256DiceRoller {
    pub fn new(secret: impl Into<Vec<u8>>) -> anyhow::Result<Self> {
        let secret = secret.into();
        anyhow::ensure!(
            secret.len() >= 32,
            "dice secret must contain at least 32 bytes"
        );
        Ok(Self { secret })
    }
}

impl DiceRollerPort for Sha256DiceRoller {
    fn roll_d20(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        expected_turn_number: i64,
        request_fingerprint: &[u8; 32],
    ) -> u8 {
        // Rejection sampling avoids modulo bias. A counter-derived digest is
        // used only in the negligible case where one digest has no byte < 240.
        for counter in 0u32.. {
            let mut hasher = Sha256::new();
            hasher.update(b"novelworld/d20/v1");
            hasher.update((self.secret.len() as u64).to_be_bytes());
            hasher.update(&self.secret);
            hasher.update(user_id.as_bytes());
            hasher.update(novel_id.as_bytes());
            hasher.update(expected_turn_number.to_be_bytes());
            hasher.update(request_fingerprint);
            hasher.update(counter.to_be_bytes());
            for byte in hasher.finalize() {
                if byte < 240 {
                    return byte % 20 + 1;
                }
            }
        }
        unreachable!("a SHA-256 byte below 240 is eventually produced")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolls_are_bounded_and_retry_stable() {
        let roller = Sha256DiceRoller::new(vec![7; 32]).unwrap();
        let user = Uuid::new_v4();
        let novel = Uuid::new_v4();
        let fingerprint = [9; 32];
        let first = roller.roll_d20(user, novel, 3, &fingerprint);
        assert!((1..=20).contains(&first));
        assert_eq!(first, roller.roll_d20(user, novel, 3, &fingerprint));
    }
}
