use super::*;

#[test]
fn long_lists_and_small_windows_stay_bounded() {
    assert_eq!(dimensions(1200.0, 900.0, 10000), (380.0, 480.0, 330.0));
    assert_eq!(dimensions(320.0, 300.0, 10000), (288.0, 268.0, 118.0));
    assert_eq!(dimensions(1200.0, 900.0, 2), (380.0, 480.0, 64.0));
    assert_eq!(dimensions(20.0, 20.0, 10000), (0.0, 0.0, 0.0));
}
