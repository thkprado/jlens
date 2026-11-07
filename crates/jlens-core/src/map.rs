//! The fields of an object, in insertion order (ADR 0022).
//!
//! Objects are small: those the benchmark workload builds, converts and reads have at most 6
//! fields, the test suite's datasets at most 11. For a few fields, comparing the key with each
//! field's key in turn costs less than hashing it, and a map without a hash table takes one
//! allocation instead of two. So a [`Map`] is a vector of fields searched in order, and only a
//! map of more than [`INDEXED_PAST`] fields keeps an index of its keys, so that an object of
//! thousands of fields (a document whose keys are identifiers) stays linear to build and read.
//!
//! The operations are those of `IndexMap` that jlens uses, with the same meaning: a key inserted
//! again keeps its position and its first key, and `shift_remove` keeps the order of the others.

use std::hash::BuildHasher;
use std::rc::Rc;

use hashbrown::HashTable;

/// Past this many fields, a map keeps an index of its keys. Measured on 0.31 #2: up to 8 fields,
/// a search in order finds a key present as fast as or faster than foldhash, and one absent within
/// about a nanosecond; it also builds the map faster, up to about 24 fields.
pub(crate) const INDEXED_PAST: usize = 8;

/// The fields of an object, in insertion order, with their shared keys.
#[derive(Clone)]
pub(crate) struct Map<V> {
    entries: Vec<(Rc<str>, V)>,
    /// The position of each key in `entries`, kept while there are more than [`INDEXED_PAST`].
    index: Option<Box<Index>>,
}

/// The index of a large map: the positions of its entries, in a table hashed by their keys.
#[derive(Clone)]
struct Index {
    table: HashTable<usize>,
    hasher: foldhash::fast::RandomState,
}

impl Index {
    #[inline]
    fn hash(&self, key: &str) -> u64 {
        self.hasher.hash_one(key)
    }

    /// The index of `entries`, built whole.
    fn of<V>(entries: &[(Rc<str>, V)]) -> Box<Index> {
        let mut index = Box::new(Index {
            table: HashTable::with_capacity(entries.len()),
            hasher: foldhash::fast::RandomState::default(),
        });
        for (i, (key, _)) in entries.iter().enumerate() {
            let hash = index.hash(key);
            let hasher = &index.hasher;
            index
                .table
                .insert_unique(hash, i, |&j| hasher.hash_one(&*entries[j].0));
        }
        index
    }

    /// The position of `key` in `entries`.
    #[inline]
    fn find<V>(&self, entries: &[(Rc<str>, V)], key: &str) -> Option<usize> {
        let hash = self.hash(key);
        self.table.find(hash, |&i| *entries[i].0 == *key).copied()
    }

    /// Removes the position `at` of `key` from the table.
    fn forget(&mut self, key: &str, at: usize) {
        let hash = self.hash(key);
        if let Ok(slot) = self.table.find_entry(hash, |&i| i == at) {
            slot.remove();
        }
    }
}

impl<V> Default for Map<V> {
    fn default() -> Self {
        Map::new()
    }
}

impl<V> Map<V> {
    /// An empty map, which allocates nothing.
    #[inline]
    pub(crate) const fn new() -> Self {
        Map {
            entries: Vec::new(),
            index: None,
        }
    }

    /// An empty map with room for `capacity` fields.
    #[inline]
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Map {
            entries: Vec::with_capacity(capacity),
            index: None,
        }
    }

    /// A copy of the map with room for `additional` more fields, so that adding them does not
    /// move its fields again, as adding to a clone does.
    pub(crate) fn cloned_with_room(&self, additional: usize) -> Self
    where
        V: Clone,
    {
        let mut entries = Vec::with_capacity(self.entries.len() + additional);
        entries.extend_from_slice(&self.entries);
        Map {
            entries,
            index: self.index.clone(),
        }
    }

    /// The number of fields.
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the map has no field.
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The position of `key`.
    #[inline]
    pub(crate) fn get_index_of(&self, key: &str) -> Option<usize> {
        match &self.index {
            None => self
                .entries
                .iter()
                .position(|(k, _)| k.len() == key.len() && **k == *key),
            Some(index) => index.find(&self.entries, key),
        }
    }

    /// The value of `key`.
    #[inline]
    pub(crate) fn get(&self, key: &str) -> Option<&V> {
        match &self.index {
            None => self
                .entries
                .iter()
                .find(|(k, _)| k.len() == key.len() && **k == *key)
                .map(|(_, v)| v),
            Some(index) => index.find(&self.entries, key).map(|i| &self.entries[i].1),
        }
    }

    /// The value of `key`, mutably.
    #[inline]
    pub(crate) fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        let i = self.get_index_of(key)?;
        Some(&mut self.entries[i].1)
    }

    /// Whether the map has the field `key`.
    #[inline]
    pub(crate) fn contains_key(&self, key: &str) -> bool {
        self.get_index_of(key).is_some()
    }

    /// Sets the field `key` to `value`. A key already present keeps its position and its key,
    /// and its former value is returned; a new key goes last.
    pub(crate) fn insert(&mut self, key: Rc<str>, value: V) -> Option<V> {
        if let Some(i) = self.get_index_of(&key) {
            return Some(std::mem::replace(&mut self.entries[i].1, value));
        }
        self.push(key, value);
        None
    }

    /// Adds the field `key`, last, unless the map has it: then nothing changes, and the key and
    /// value are given back.
    pub(crate) fn insert_new(&mut self, key: Rc<str>, value: V) -> Result<(), (Rc<str>, V)> {
        if self.contains_key(&key) {
            return Err((key, value));
        }
        self.push(key, value);
        Ok(())
    }

    /// Adds a field whose key the map does not have.
    fn push(&mut self, key: Rc<str>, value: V) {
        let at = self.entries.len();
        match &mut self.index {
            Some(index) => {
                let hash = index.hash(&key);
                let (entries, hasher) = (&self.entries, &index.hasher);
                index
                    .table
                    .insert_unique(hash, at, |&j| hasher.hash_one(&*entries[j].0));
                self.entries.push((key, value));
            }
            None => {
                self.entries.push((key, value));
                if self.entries.len() > INDEXED_PAST {
                    self.index = Some(Index::of(&self.entries));
                }
            }
        }
    }

    /// Removes the field `key`, keeping the order of the others, and returns its value.
    pub(crate) fn shift_remove(&mut self, key: &str) -> Option<V> {
        let i = self.get_index_of(key)?;
        let (_, value) = self.entries.remove(i);
        if let Some(index) = &mut self.index {
            if self.entries.len() <= INDEXED_PAST {
                self.index = None;
            } else {
                index.forget(key, i);
                for j in index.table.iter_mut() {
                    if *j > i {
                        *j -= 1;
                    }
                }
            }
        }
        Some(value)
    }

    /// Removes the field `key` and puts the last field in its place, and returns its value.
    pub(crate) fn swap_remove(&mut self, key: &str) -> Option<V> {
        let i = self.get_index_of(key)?;
        let (_, value) = self.entries.swap_remove(i);
        if let Some(index) = &mut self.index {
            if self.entries.len() <= INDEXED_PAST {
                self.index = None;
            } else {
                index.forget(key, i);
                // The last field, now at `i`, was at `len`.
                if let Some((moved, _)) = self.entries.get(i) {
                    let last = self.entries.len();
                    let hash = index.hash(moved);
                    if let Some(j) = index.table.find_mut(hash, |&j| j == last) {
                        *j = i;
                    }
                }
            }
        }
        Some(value)
    }

    /// Keeps the fields for which `keep` is true, in their order.
    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&Rc<str>, &mut V) -> bool) {
        let len = self.entries.len();
        self.entries.retain_mut(|(k, v)| keep(k, v));
        if self.entries.len() != len {
            self.reindex();
        }
    }

    /// Builds the index again after the entries moved, or drops it if the map got small.
    fn reindex(&mut self) {
        self.index = match self.entries.len() > INDEXED_PAST {
            true => Some(Index::of(&self.entries)),
            false => None,
        };
    }

    /// Removes every field, keeping the room of the entries.
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.index = None;
    }

    /// Makes room for `additional` more fields.
    pub(crate) fn reserve(&mut self, additional: usize) {
        self.entries.reserve(additional);
    }

    /// The fields, in order.
    #[inline]
    pub(crate) fn iter(&self) -> Iter<'_, V> {
        Iter(self.entries.iter())
    }

    /// The fields, in order, with their values mutable.
    #[inline]
    pub(crate) fn iter_mut(&mut self) -> IterMut<'_, V> {
        IterMut(self.entries.iter_mut())
    }

    /// The keys, in order.
    #[inline]
    pub(crate) fn keys(&self) -> impl DoubleEndedIterator<Item = &Rc<str>> + ExactSizeIterator {
        self.entries.iter().map(|(k, _)| k)
    }

    /// The values, in order.
    #[inline]
    pub(crate) fn values(&self) -> impl DoubleEndedIterator<Item = &V> + ExactSizeIterator {
        self.entries.iter().map(|(_, v)| v)
    }

    /// The values, in order, taken from the map.
    #[inline]
    pub(crate) fn into_values(self) -> impl DoubleEndedIterator<Item = V> + ExactSizeIterator {
        self.entries.into_iter().map(|(_, v)| v)
    }
}

/// The fields of a [`Map`], in order.
#[derive(Clone)]
pub(crate) struct Iter<'m, V>(std::slice::Iter<'m, (Rc<str>, V)>);

impl<'m, V> Iterator for Iter<'m, V> {
    type Item = (&'m Rc<str>, &'m V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(k, v)| (k, v))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<V> DoubleEndedIterator for Iter<'_, V> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(|(k, v)| (k, v))
    }
}

impl<V> ExactSizeIterator for Iter<'_, V> {}
impl<V> std::iter::FusedIterator for Iter<'_, V> {}

/// The fields of a [`Map`], in order, with their values mutable.
pub(crate) struct IterMut<'m, V>(std::slice::IterMut<'m, (Rc<str>, V)>);

impl<'m, V> Iterator for IterMut<'m, V> {
    type Item = (&'m Rc<str>, &'m mut V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(k, v)| (&*k, v))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<V> DoubleEndedIterator for IterMut<'_, V> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(|(k, v)| (&*k, v))
    }
}

impl<V> ExactSizeIterator for IterMut<'_, V> {}
impl<V> std::iter::FusedIterator for IterMut<'_, V> {}

impl<'m, V> IntoIterator for &'m Map<V> {
    type Item = (&'m Rc<str>, &'m V);
    type IntoIter = Iter<'m, V>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'m, V> IntoIterator for &'m mut Map<V> {
    type Item = (&'m Rc<str>, &'m mut V);
    type IntoIter = IterMut<'m, V>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl<V> IntoIterator for Map<V> {
    type Item = (Rc<str>, V);
    type IntoIter = std::vec::IntoIter<(Rc<str>, V)>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}

impl<V> Extend<(Rc<str>, V)> for Map<V> {
    fn extend<I: IntoIterator<Item = (Rc<str>, V)>>(&mut self, fields: I) {
        let fields = fields.into_iter();
        // As `IndexMap` does: room for all of them in an empty map, for half of them otherwise,
        // since some keys may be present.
        let (lower, _) = fields.size_hint();
        self.reserve(match self.is_empty() {
            true => lower,
            false => lower.div_ceil(2),
        });
        for (key, value) in fields {
            self.insert(key, value);
        }
    }
}

impl<V> FromIterator<(Rc<str>, V)> for Map<V> {
    fn from_iter<I: IntoIterator<Item = (Rc<str>, V)>>(fields: I) -> Self {
        let mut map = Map::new();
        map.extend(fields);
        map
    }
}

impl<V> std::ops::Index<&str> for Map<V> {
    type Output = V;

    /// The value of `key`; panics when the map does not have it, as `IndexMap` does. For tests
    /// and fields known to be there.
    fn index(&self, key: &str) -> &V {
        self.get(key).expect("a field of the map")
    }
}

impl<V> std::ops::Index<usize> for Map<V> {
    type Output = V;

    /// The value of the field at position `i`; panics past the last field, as `IndexMap` does.
    fn index(&self, i: usize) -> &V {
        &self.entries[i].1
    }
}

impl<V> std::ops::IndexMut<usize> for Map<V> {
    /// The value of the field at position `i`; panics past the last field, as `IndexMap` does.
    fn index_mut(&mut self, i: usize) -> &mut V {
        &mut self.entries[i].1
    }
}

impl<V: std::fmt::Debug> std::fmt::Debug for Map<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;
    use proptest::prelude::*;

    fn key(k: &str) -> Rc<str> {
        Rc::from(k)
    }

    /// The map's fields in order, and checks that its index agrees with them.
    fn fields<V: Clone>(map: &Map<V>) -> Vec<(String, V)> {
        assert_eq!(map.index.is_some(), map.len() > INDEXED_PAST);
        if let Some(index) = &map.index {
            assert_eq!(index.table.len(), map.len());
            for (i, (k, _)) in map.entries.iter().enumerate() {
                assert_eq!(index.find(&map.entries, k), Some(i));
            }
        }
        map.iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn a_key_inserted_again_keeps_its_position_and_first_key() {
        let mut map = Map::new();
        let first = key("a");
        assert_eq!(map.insert(first.clone(), 1), None);
        map.insert(key("b"), 2);
        assert_eq!(map.insert(key("a"), 3), Some(1));
        assert_eq!(fields(&map), [("a".into(), 3), ("b".into(), 2)]);
        assert!(Rc::ptr_eq(map.keys().next().unwrap(), &first));
    }

    #[test]
    fn a_large_map_keeps_an_index_and_drops_it_when_small_again() {
        let names: Vec<String> = (0..20).map(|i| format!("k{i}")).collect();
        let mut map: Map<usize> = names.iter().map(|n| key(n)).zip(0..).collect();
        assert!(map.index.is_some());
        assert_eq!(map.get("k13"), Some(&13));
        assert_eq!(map.get("k20"), None);
        for n in &names[..12] {
            map.shift_remove(n);
            fields(&map);
        }
        assert!(map.index.is_none());
        assert_eq!(map.get("k19"), Some(&19));
    }

    #[test]
    fn removals_keep_or_swap_the_order() {
        let mut map: Map<i32> = ["a", "b", "c", "d"].map(key).into_iter().zip(0..).collect();
        assert_eq!(map.shift_remove("b"), Some(1));
        assert_eq!(
            map.keys().map(|k| &**k).collect::<Vec<_>>(),
            ["a", "c", "d"]
        );
        assert_eq!(map.swap_remove("a"), Some(0));
        assert_eq!(map.keys().map(|k| &**k).collect::<Vec<_>>(), ["d", "c"]);
        assert_eq!(map.shift_remove("x"), None);
    }

    #[derive(Debug, Clone)]
    enum Op {
        Insert(usize, u32),
        ShiftRemove(usize),
        SwapRemove(usize),
        GetMut(usize, u32),
        Clear,
        Retain(u32),
        InsertNew(usize, u32),
        ClonedWithRoom(usize),
        Extend(Vec<(usize, u32)>),
    }

    /// More keys than [`INDEXED_PAST`], so that maps cross it both ways; some of the keys have
    /// the same length.
    const KEYS: usize = 24;

    fn op() -> impl Strategy<Value = Op> {
        let k = 0..KEYS;
        prop_oneof![
            6 => (k.clone(), any::<u32>()).prop_map(|(k, v)| Op::Insert(k, v)),
            2 => k.clone().prop_map(Op::ShiftRemove),
            2 => k.clone().prop_map(Op::SwapRemove),
            1 => (k.clone(), any::<u32>()).prop_map(|(k, v)| Op::GetMut(k, v)),
            1 => Just(Op::Clear),
            1 => any::<u32>().prop_map(Op::Retain),
            1 => (0..4usize).prop_map(Op::ClonedWithRoom),
            2 => (k.clone(), any::<u32>()).prop_map(|(k, v)| Op::InsertNew(k, v)),
            1 => prop::collection::vec((k, any::<u32>()), 0..12).prop_map(Op::Extend),
        ]
    }

    proptest! {
        /// Random operations give the same answers and the same fields, in the same order, as
        /// `IndexMap`.
        #[test]
        fn behaves_as_index_map(ops in prop::collection::vec(op(), 0..200)) {
            let names: Vec<Rc<str>> = (0..KEYS).map(|i| key(&format!("{}", i * 7))).collect();
            let mut map: Map<u32> = Map::new();
            let mut model: IndexMap<Rc<str>, u32> = IndexMap::new();
            for op in ops {
                match op {
                    Op::Insert(k, v) => {
                        prop_assert_eq!(map.insert(names[k].clone(), v), model.insert(names[k].clone(), v));
                    }
                    Op::ShiftRemove(k) => {
                        prop_assert_eq!(map.shift_remove(&names[k]), model.shift_remove(&names[k]));
                    }
                    Op::SwapRemove(k) => {
                        prop_assert_eq!(map.swap_remove(&names[k]), model.swap_remove(&names[k]));
                    }
                    Op::GetMut(k, v) => {
                        let (a, b) = (map.get_mut(&names[k]), model.get_mut(&names[k]));
                        prop_assert_eq!(a.is_some(), b.is_some());
                        if let (Some(a), Some(b)) = (a, b) {
                            (*a, *b) = (v, v);
                        }
                    }
                    Op::Clear => {
                        map.clear();
                        model.clear();
                    }
                    Op::Retain(bits) => {
                        // Keeps the fields whose value has the bit of `bits` that its key names.
                        let keep = |k: &Rc<str>, v: &mut u32| (*v ^ bits) >> (k.len() % 32) & 1 == 0;
                        map.retain(keep);
                        model.retain(keep);
                    }
                    Op::ClonedWithRoom(room) => {
                        map = map.cloned_with_room(room);
                        prop_assert!(map.entries.capacity() >= map.len() + room);
                    }
                    Op::InsertNew(k, v) => {
                        let had = model.contains_key(&names[k]);
                        if !had {
                            model.insert(names[k].clone(), v);
                        }
                        prop_assert_eq!(map.insert_new(names[k].clone(), v).is_err(), had);
                    }
                    Op::Extend(fields) => {
                        let fields: Vec<_> = fields.into_iter().map(|(k, v)| (names[k].clone(), v)).collect();
                        map.extend(fields.clone());
                        model.extend(fields);
                    }
                }
                let expected: Vec<(String, u32)> = model.iter().map(|(k, v)| (k.to_string(), *v)).collect();
                prop_assert_eq!(fields(&map), expected);
                for name in &names {
                    prop_assert_eq!(map.get(name), model.get(name));
                    prop_assert_eq!(map.get_index_of(name), model.get_index_of(name));
                }
            }
        }
    }
}
