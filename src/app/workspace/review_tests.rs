use super::*;
use crate::app::reviews::{ReviewLocation, ReviewLocationStatus};

fn active() -> ActiveReview {
    ActiveReview::new(
        7,
        "session:a".into(),
        "/project".into(),
        Review {
            title: "Review".into(),
            items: vec![ReviewLocation {
                path: "file.rs".into(),
                start_line: Some(1),
                end_line: None,
                note: "Inspect".into(),
            }],
        },
    )
}

#[test]
fn sidebar_is_scoped_to_the_editors_session() {
    let review = active();
    assert!(review.is_visible(AppSurface::Editor, "session:a"));
    assert!(!review.is_visible(AppSurface::Editor, "session:b"));
    for surface in [AppSurface::Chat, AppSurface::Terminal, AppSurface::Work] {
        assert!(!review.is_visible(surface, "session:a"));
    }
}

#[test]
fn unavailable_location_keeps_its_details_inspectable() {
    let mut review = active();
    assert!(review.complete(
        7,
        Ok(ReviewNavigation {
            list_id: 42,
            selected: None,
            locations: vec![ReviewLocationStatus {
                valid: false,
                warning: Some("Missing file".into())
            }],
        })
    ));
    assert_eq!(review.inspecting, Some(0));
    assert_eq!(review.navigation.as_ref().unwrap().selected, None);
    review.pending = Some(8);
    assert!(review.complete(8, Err("List was removed".into())));
    assert_eq!(review.inspecting, Some(0));
}

#[test]
fn late_navigation_cannot_overwrite_a_newer_request() {
    let mut review = active();
    assert!(!review.complete(6, Err("old request".into())));
    assert_eq!(review.pending, Some(7));
    assert!(review.error.is_none());
    let navigation = ReviewNavigation {
        list_id: 42,
        selected: Some(0),
        locations: vec![ReviewLocationStatus {
            valid: true,
            warning: None,
        }],
    };
    assert!(review.complete(7, Ok(navigation)));
    assert_eq!(review.navigation.as_ref().unwrap().selected, Some(0));
    review.pending = Some(8);
    assert!(!review.complete(7, Err("late failure".into())));
    assert!(review.complete(8, Err("List was removed; reopen the review".into())));
    assert!(review.pending.is_none());
    assert!(review.error.is_some());
    assert_eq!(review.navigation.as_ref().unwrap().list_id, 42);
}
