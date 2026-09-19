use super::*;

#[test]
fn token_usage_has_one_shared_saturating_accounting_rule() {
    let first = TokenUsage {
        input: 100,
        output: 20,
        cache_read: 80,
        cache_write: 10,
    };
    let total = first.saturating_add(TokenUsage {
        input: 50,
        output: 5,
        cache_read: 40,
        cache_write: 0,
    });
    assert_eq!(first.total(), 210);
    assert_eq!(total.input, 150);
    assert_eq!(total.output, 25);
    assert_eq!(total.cache_read, 120);
    assert_eq!(total.total(), 305);
}
