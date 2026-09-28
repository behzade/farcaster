use super::*;

#[test]
fn activity_is_project_scoped_retains_short_runs_and_cleans_up() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("one");
    let other = directory.path().join("two");
    let subscription = subscribe_worker_activity(&project, std::thread::current());
    let unrelated = subscribe_worker_activity(&other, std::thread::current());
    record(&project);
    record(&project);
    assert_eq!(subscription.revision(), 2);
    assert_eq!(unrelated.revision(), 0);
    drop(subscription);
    assert!(!subscribers().lock().unwrap().contains_key(&project));
    record(&project);
    assert!(!subscribers().lock().unwrap().contains_key(&project));
}

#[test]
fn activity_wakes_a_parked_subscriber() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().to_owned();
    let watched = project.clone();
    let (armed, ready) = std::sync::mpsc::channel();
    let (sent, received) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let subscription = subscribe_worker_activity(&watched, std::thread::current());
        armed.send(()).unwrap();
        while subscription.revision() == 0 {
            std::thread::park();
        }
        sent.send(subscription.revision()).unwrap();
    });
    ready.recv().unwrap();
    record(&project);
    assert_eq!(
        received
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap(),
        1
    );
    thread.join().unwrap();
}
