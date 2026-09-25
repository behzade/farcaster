use super::*;

#[test]
fn the_flag_is_read_from_any_position_and_leaves_one_project() {
    let arguments = [
        OsString::from("--isolated"),
        OsString::from("/projects/app"),
    ];
    let (project, isolated) = split(arguments.into_iter());
    assert!(isolated);
    assert_eq!(project, Some(PathBuf::from("/projects/app")));

    let arguments = [
        OsString::from("/projects/app"),
        OsString::from("--isolated"),
        OsString::from("/projects/other"),
    ];
    let (project, isolated) = split(arguments.into_iter());
    assert!(isolated);
    assert_eq!(project, Some(PathBuf::from("/projects/app")));

    let (project, isolated) = split([OsString::from("/projects/app")].into_iter());
    assert!(!isolated);
    assert_eq!(project, Some(PathBuf::from("/projects/app")));
}
