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
