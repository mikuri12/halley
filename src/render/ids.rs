use std::collections::HashMap;
use std::hash::Hash;

use smithay::backend::renderer::element::Id;

/// How many frames an unused identity is kept before it is dropped.
///
/// Overlays fade in and out constantly, so an identity that vanishes for a
/// frame or two should keep its `Id` rather than churn the damage tracker.
const RETAIN_FRAMES: u64 = 120;

/// How often the sweep runs, in frames. Sweeping every frame would walk the
/// whole map in the hot path for no benefit.
const SWEEP_INTERVAL: u64 = 240;

struct Entry {
    id: Id,
    last_seen: u64,
}

/// Stable render-element identities, keyed by logical slot.
///
/// Smithay's `OutputDamageTracker` keys its per-element state by `Id`. Minting
/// a fresh `Id::new()` every frame makes each element look brand new, so the
/// tracker damages its full geometry twice — once as "added", once as the
/// previous frame's element "gone" — and, because *something* went away, sets
/// `force_effect_redraw`, which re-captures and re-blurs the entire framebuffer
/// for every backdrop-blur element. Handing out an `Id` that is stable across
/// frames is what keeps damage proportional to what actually changed.
///
/// Identities must name the **slot**, not the content: the damage tracker
/// compares `last_z_index` alongside geometry, so two elements that can swap
/// draw order need distinct keys.
pub struct ElementIds<K> {
    entries: HashMap<K, Entry>,
    generation: u64,
}

impl<K> Default for ElementIds<K> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            generation: 0,
        }
    }
}

impl<K: Eq + Hash> ElementIds<K> {
    /// Returns the stable `Id` for `key`, creating one on first use.
    pub fn id(&mut self, key: K) -> Id {
        let generation = self.generation;
        let entry = self.entries.entry(key).or_insert_with(|| Entry {
            id: Id::new(),
            last_seen: generation,
        });
        entry.last_seen = generation;
        entry.id.clone()
    }

    /// Advances to the next frame, periodically dropping identities that have
    /// gone unused for [`RETAIN_FRAMES`].
    pub fn advance(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if self.generation.is_multiple_of(SWEEP_INTERVAL) {
            let cutoff = self.generation.saturating_sub(RETAIN_FRAMES);
            self.entries.retain(|_, entry| entry.last_seen >= cutoff);
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Per-output [`ElementIds`], so one renderer can serve several outputs
/// without their slots colliding.
///
/// Outputs are looked up by name once per frame; the per-slot keys are then
/// cheap `Copy` values, which keeps the hot path allocation-free.
pub struct OutputElementIds<K> {
    outputs: HashMap<String, ElementIds<K>>,
}

impl<K> Default for OutputElementIds<K> {
    fn default() -> Self {
        Self {
            outputs: HashMap::new(),
        }
    }
}

impl<K: Eq + Hash> OutputElementIds<K> {
    pub fn for_output(&mut self, output: &str) -> &mut ElementIds<K> {
        // `get_mut` first so the common path never allocates the key.
        if !self.outputs.contains_key(output) {
            self.outputs
                .insert(output.to_string(), ElementIds::default());
        }
        self.outputs.get_mut(output).expect("inserted above")
    }

    pub fn advance(&mut self, output: &str) {
        self.for_output(output).advance();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_slot_keeps_one_id_across_frames() {
        let mut ids = ElementIds::default();
        let first = ids.id(7u32);
        ids.advance();

        assert_eq!(ids.id(7u32), first);
    }

    #[test]
    fn distinct_slots_get_distinct_ids() {
        let mut ids = ElementIds::default();

        assert_ne!(ids.id(1u32), ids.id(2u32));
    }

    #[test]
    fn unused_identities_are_swept_but_survive_a_brief_absence() {
        let mut ids = ElementIds::default();
        let kept = ids.id(1u32);
        ids.id(2u32);

        // Well inside the retention window: both survive, and the identity
        // that keeps being used is unchanged.
        for _ in 0..RETAIN_FRAMES {
            ids.advance();
            ids.id(1u32);
        }
        assert_eq!(ids.len(), 2);
        assert_eq!(ids.id(1u32), kept);

        // Past the sweep boundary the abandoned slot is released.
        while !ids.generation.is_multiple_of(SWEEP_INTERVAL) {
            ids.advance();
            ids.id(1u32);
        }
        assert_eq!(ids.len(), 1);
        assert_eq!(ids.id(1u32), kept);
    }

    #[test]
    fn outputs_do_not_share_slots() {
        let mut ids = OutputElementIds::default();
        let primary = ids.for_output("DP-1").id(1u32);
        let secondary = ids.for_output("DP-2").id(1u32);

        assert_ne!(primary, secondary);
        assert_eq!(ids.for_output("DP-1").id(1u32), primary);
    }
}
