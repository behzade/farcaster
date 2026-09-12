use gpui::{ImageSource, Resource};

use crate::text::utils::{image_source, list_item_prefix};

#[test]
fn test_image_source() {
    fn source(url: &str) -> Resource {
        match image_source(&url.to_string().into()) {
            ImageSource::Resource(resource) => resource,
            _ => panic!("expected a resource for {url:?}"),
        }
    }
    fn assert_uri(url: &str) {
        match source(url) {
            Resource::Uri(uri) => assert_eq!(uri.as_ref(), url),
            other => panic!("expected Uri for {url:?}, got {other:?}"),
        }
    }
    assert_uri("https://example.com/logo.png");
    assert_uri("http://example.com/logo.png");
    assert_uri("data:image/png;base64,iVBORw0KGgo=");

    assert_uri("website/public/logo.svg");
    assert_uri("./images/a.png");
    assert_uri("../images/a.png");
    assert_uri("/absolute/path/logo.svg");
    assert_uri("file:///absolute/path/logo.svg");
    assert_uri(r"C:\images\logo.png");
    assert_uri("docs/a:b.png");
}

#[test]
fn test_list_item_prefix() {
    assert_eq!(list_item_prefix(0, true, 0, 1), "1. ");
    assert_eq!(list_item_prefix(1, true, 0, 1), "2. ");
    assert_eq!(list_item_prefix(2, true, 0, 1), "3. ");
    assert_eq!(list_item_prefix(10, true, 0, 1), "11. ");
    assert_eq!(list_item_prefix(0, true, 1, 1), "A. ");
    assert_eq!(list_item_prefix(1, true, 1, 1), "B. ");
    assert_eq!(list_item_prefix(2, true, 1, 1), "C. ");
    assert_eq!(list_item_prefix(0, true, 2, 1), "a. ");
    assert_eq!(list_item_prefix(1, true, 2, 1), "b. ");
    assert_eq!(list_item_prefix(6, true, 2, 1), "g. ");
    assert_eq!(list_item_prefix(0, false, 0, 1), "• ");
    assert_eq!(list_item_prefix(0, false, 1, 1), "◦ ");
    assert_eq!(list_item_prefix(0, false, 2, 1), "▪ ");
    assert_eq!(list_item_prefix(0, false, 3, 1), "‣ ");
    assert_eq!(list_item_prefix(0, false, 4, 1), "⁃ ");
}

#[test]
fn ordered_prefixes_apply_the_start_without_changing_marker_styles() {
    assert_eq!(list_item_prefix(0, true, 0, 0), "0. ");
    assert_eq!(list_item_prefix(1, true, 0, 0), "1. ");
    assert_eq!(list_item_prefix(0, true, 0, 9), "9. ");
    assert_eq!(list_item_prefix(1, true, 0, 9), "10. ");
    assert_eq!(list_item_prefix(0, true, 1, 3), "C. ");
    assert_eq!(list_item_prefix(0, true, 2, 3), "c. ");
    assert_eq!(list_item_prefix(0, false, 0, 9), "• ");
    assert_eq!(list_item_prefix(1, true, 0, u32::MAX), "4294967296. ");
}
