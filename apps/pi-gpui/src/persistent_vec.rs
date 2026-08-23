//! Structurally shared sequence storage for immutable runtime snapshots.

use std::{
    fmt,
    ops::{Index, Range},
};

use sum_tree::{Bias, ContextLessSummary, Dimension, Item, SumTree};

#[derive(Clone, Default)]
pub(crate) struct CountSummary {
    count: usize,
}

impl ContextLessSummary for CountSummary {
    fn zero() -> Self {
        Self::default()
    }

    fn add_summary(&mut self, summary: &Self) {
        self.count += summary.count;
    }
}

#[derive(Clone, Copy, Default, Eq, Ord, PartialEq, PartialOrd)]
struct Count(usize);

impl Dimension<'_, CountSummary> for Count {
    fn zero(_: ()) -> Self {
        Self::default()
    }

    fn add_summary(&mut self, summary: &CountSummary, _: ()) {
        self.0 += summary.count;
    }
}

#[derive(Clone)]
pub(crate) struct Entry<T: Clone>(T);

impl<T: Clone> Item for Entry<T> {
    type Summary = CountSummary;

    fn summary(&self, _: ()) -> Self::Summary {
        CountSummary { count: 1 }
    }
}

pub(crate) trait Indexed<T> {
    fn len(&self) -> usize;
    fn get(&self, index: usize) -> Option<&T>;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T> Indexed<T> for [T] {
    fn len(&self) -> usize {
        <[T]>::len(self)
    }

    fn get(&self, index: usize) -> Option<&T> {
        <[T]>::get(self, index)
    }
}

impl<T, const N: usize> Indexed<T> for [T; N] {
    fn len(&self) -> usize {
        N
    }

    fn get(&self, index: usize) -> Option<&T> {
        self.as_slice().get(index)
    }
}

impl<T> Indexed<T> for Vec<T> {
    fn len(&self) -> usize {
        Vec::len(self)
    }

    fn get(&self, index: usize) -> Option<&T> {
        self.as_slice().get(index)
    }
}

#[derive(Clone)]
pub(crate) struct PersistentVec<T: Clone> {
    tree: SumTree<Entry<T>>,
}

impl<T: Clone> Default for PersistentVec<T> {
    fn default() -> Self {
        Self {
            tree: SumTree::new(()),
        }
    }
}

impl<T: Clone> PersistentVec<T> {
    pub(crate) fn len(&self) -> usize {
        self.tree.summary().count
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.tree.is_empty()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &T> {
        self.tree.iter().map(|entry| &entry.0)
    }

    pub(crate) fn iter_rev(&self) -> impl DoubleEndedIterator<Item = &T> + ExactSizeIterator {
        (0..self.len()).rev().map(|index| &self[index])
    }

    pub(crate) fn iter_range(
        &self,
        range: Range<usize>,
    ) -> impl DoubleEndedIterator<Item = &T> + ExactSizeIterator {
        assert!(range.start <= range.end && range.end <= self.len());
        range.map(|index| &self[index])
    }

    pub(crate) fn get(&self, index: usize) -> Option<&T> {
        if index >= self.len() {
            return None;
        }
        let mut cursor = self.tree.cursor::<Count>(());
        cursor.seek(&Count(index), Bias::Right);
        cursor.item().map(|entry| &entry.0)
    }

    #[allow(dead_code)]
    pub(crate) fn first(&self) -> Option<&T> {
        self.get(0)
    }

    pub(crate) fn last(&self) -> Option<&T> {
        self.len().checked_sub(1).and_then(|index| self.get(index))
    }

    pub(crate) fn push(&mut self, value: T) {
        self.tree.push(Entry(value), ());
    }

    pub(crate) fn extend(&mut self, values: impl IntoIterator<Item = T>) {
        self.tree.extend(values.into_iter().map(Entry), ());
    }

    pub(crate) fn clear(&mut self) {
        self.tree = SumTree::new(());
    }

    pub(crate) fn set(&mut self, index: usize, value: T) {
        assert!(index < self.len(), "persistent vector index out of bounds");
        self.splice(index..index + 1, [value]);
    }

    pub(crate) fn insert(&mut self, index: usize, value: T) {
        assert!(index <= self.len(), "persistent vector index out of bounds");
        self.splice(index..index, [value]);
    }

    pub(crate) fn remove(&mut self, index: usize) -> T {
        let value = self[index].clone();
        self.splice(index..index + 1, []);
        value
    }

    pub(crate) fn splice(&mut self, range: Range<usize>, values: impl IntoIterator<Item = T>) {
        assert!(range.start <= range.end && range.end <= self.len());
        let next = {
            let mut cursor = self.tree.cursor::<Count>(());
            let mut next = cursor.slice(&Count(range.start), Bias::Right);
            next.extend(values.into_iter().map(Entry), ());
            cursor.seek(&Count(range.end), Bias::Right);
            next.append(cursor.suffix(), ());
            next
        };
        self.tree = next;
    }

    pub(crate) fn position(&self, mut predicate: impl FnMut(&T) -> bool) -> Option<usize> {
        self.iter().position(&mut predicate)
    }

    pub(crate) fn rposition(&self, mut predicate: impl FnMut(&T) -> bool) -> Option<usize> {
        (0..self.len()).rev().find(|&index| predicate(&self[index]))
    }

    pub(crate) fn partition_point(&self, mut predicate: impl FnMut(&T) -> bool) -> usize {
        let mut left = 0;
        let mut right = self.len();
        while left < right {
            let middle = left + (right - left) / 2;
            if predicate(&self[middle]) {
                left = middle + 1;
            } else {
                right = middle;
            }
        }
        left
    }
}

impl<T: Clone> Indexed<T> for PersistentVec<T> {
    fn len(&self) -> usize {
        self.len()
    }

    fn get(&self, index: usize) -> Option<&T> {
        self.get(index)
    }
}

impl<T: Clone> FromIterator<T> for PersistentVec<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self {
            tree: SumTree::from_iter(iter.into_iter().map(Entry), ()),
        }
    }
}

impl<'a, T: Clone> IntoIterator for &'a PersistentVec<T> {
    type Item = &'a T;
    type IntoIter = std::iter::Map<sum_tree::Iter<'a, Entry<T>>, fn(&'a Entry<T>) -> &'a T>;

    fn into_iter(self) -> Self::IntoIter {
        fn value<T: Clone>(entry: &Entry<T>) -> &T {
            &entry.0
        }
        self.tree.iter().map(value::<T>)
    }
}

impl<T: Clone> Index<usize> for PersistentVec<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        self.get(index)
            .expect("persistent vector index out of bounds")
    }
}

impl<T: Clone + fmt::Debug> fmt::Debug for PersistentVec<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_list().entries(self.iter()).finish()
    }
}

impl<T: Clone + PartialEq> PartialEq for PersistentVec<T> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len()
            && self
                .iter()
                .zip(other.iter())
                .all(|(left, right)| left == right)
    }
}

impl<T: Clone + Eq> Eq for PersistentVec<T> {}

impl<T: Clone + PartialEq> PartialEq<Vec<T>> for PersistentVec<T> {
    fn eq(&self, other: &Vec<T>) -> bool {
        self.len() == other.len() && self.iter().eq(other)
    }
}

#[cfg(test)]
mod tests {
    use super::PersistentVec;

    #[test]
    fn clones_share_unchanged_prefixes_across_splices() {
        let original = (0..100).collect::<PersistentVec<_>>();
        let mut changed = original.clone();
        changed.splice(98..100, [200, 201, 202]);

        assert_eq!(
            original.iter().copied().collect::<Vec<_>>(),
            (0..100).collect::<Vec<_>>()
        );
        assert_eq!(changed.len(), 101);
        assert_eq!(changed[97], 97);
        assert_eq!(changed[98], 200);
        assert_eq!(changed[100], 202);
    }
}
