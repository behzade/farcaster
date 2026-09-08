use super::file_label;
use std::path::Path;

#[test]
fn file_labels_prefer_project_then_home() {
    let project = Some(Path::new("/home/user/repo"));
    let home = Some(Path::new("/home/user"));
    for (path, expected) in [
        ("/home/user/repo/src/main.rs", "src/main.rs"),
        ("src/main.rs", "src/main.rs"),
        ("../notes.txt", "~/notes.txt"),
        ("/home/user/repo-other/file.rs", "~/repo-other/file.rs"),
        ("/home/user-other/file.rs", "/home/user-other/file.rs"),
        ("/etc/config", "/etc/config"),
    ] {
        assert_eq!(file_label(path, project, home), expected, "{path}");
    }
}
