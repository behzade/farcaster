use crate::{
    Backend,
    HarnessAccessMode::{Auto, Full, Sandboxed},
    available_access_modes, delegated_worker_access_mode, worker_access_modes,
};

#[test]
fn worker_delegation_preserves_modes_except_pi_parent_containment() {
    for backend in Backend::ALL {
        for mode in [Auto, Full, Sandboxed] {
            let expected = if backend == Backend::Pi && mode == Sandboxed {
                Auto
            } else {
                mode
            };
            assert_eq!(delegated_worker_access_mode(backend, mode), expected);
        }
    }
}

#[test]
fn restricted_workers_require_native_containment_evidence() {
    assert_eq!(available_access_modes(Backend::Pi, None, None), [Auto]);
    assert!(worker_access_modes(Backend::Pi, None, None).is_empty());
    assert!(worker_access_modes(Backend::Pi, None, Some("unknown")).is_empty());
    assert_eq!(
        worker_access_modes(Backend::Pi, None, Some("pi-nono")),
        [Sandboxed, Full]
    );

    let mut model: crate::extensions::Model = serde_json::from_value(serde_json::json!({
        "id": "model", "name": "Model", "provider": "provider"
    }))
    .unwrap();
    for modes in [vec![Sandboxed, Full], vec![Sandboxed, Auto, Full]] {
        model.access_modes = Some(modes);
        for backend in Backend::ALL
            .into_iter()
            .filter(|backend| *backend != Backend::Pi)
        {
            for selection in [None, Some(&model)] {
                assert_eq!(
                    worker_access_modes(backend, selection, None),
                    available_access_modes(backend, selection, None),
                    "{backend}"
                );
            }
        }
    }
}
