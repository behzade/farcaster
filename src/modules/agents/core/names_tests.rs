use super::*;

#[test]
fn generated_names_have_a_large_namespace_and_valid_shape() {
    assert!(ADJECTIVES.len() * ANIMALS.len() >= 16_000);
    let name = generated_name(|_| false);
    assert!(crate::agents::valid_worker_name(&name));
    assert!(name.bytes().all(|byte| byte.is_ascii_alphanumeric()));
}

#[test]
fn generation_skips_occupied_names() {
    let first = generated_name(|_| false);
    let second = generated_name(|candidate| candidate.eq_ignore_ascii_case(&first));
    assert_ne!(first, second);
}

#[test]
fn worker_chosen_names_are_bounded_identifiers() {
    assert!(crate::agents::valid_worker_name("diff-review"));
    assert!(crate::agents::valid_worker_name("Auth_Tests2"));
    assert!(!crate::agents::valid_worker_name(""));
    assert!(!crate::agents::valid_worker_name("two words"));
    assert!(!crate::agents::valid_worker_name("-review"));
    assert!(!crate::agents::valid_worker_name(&"a".repeat(49)));
}
