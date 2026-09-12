use super::*;
use crate::IconName;

#[gpui::test]
fn test_button_icon_builder(_cx: &mut gpui::TestAppContext) {
    let custom_icon = Icon::new(IconName::Loader);
    let icon = ButtonIcon::new(IconName::Plus)
        .loading(true)
        .loading_icon(Some(custom_icon))
        .large();

    assert!(icon.loading);
    assert!(icon.loading_icon.is_some());
    assert_eq!(icon.size, Size::Large);
}

#[gpui::test]
fn test_button_icon_variant_types(_cx: &mut gpui::TestAppContext) {
    // Test Icon variant
    let icon_variant = ButtonIconVariant::Icon(Icon::new(IconName::Plus));
    assert!(!icon_variant.is_spinner());

    // Test Spinner variant
    let spinner_variant = ButtonIconVariant::Spinner(Spinner::new());
    assert!(spinner_variant.is_spinner());
}
