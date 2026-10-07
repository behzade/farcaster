use super::*;

#[test]
fn sdk_timings_measure_awaited_work_preserve_errors_and_omit_payloads() {
    let output = std::process::Command::new("node")
        .args([
            "--input-type=module",
            "-e",
            &script(
                r#"
const secret = 'credential-and-prompt-must-not-be-logged';
const value = await timed('Agent.send', async () => {
  await new Promise(resolve => setTimeout(resolve, 25));
  return secret;
});
if (value !== secret) throw Error('result changed');
const failure = new Error(secret);
try {
  await timed('Run.wait', async () => { throw failure; });
  throw Error('rejection lost');
} catch (error) { if (error !== failure) throw error; }
if (timedSync('Cursor.configure', () => 42) !== 42) throw Error('sync result changed');
try { timedSync('Agent.close', () => { throw failure; }); }
catch (error) { if (error !== failure) throw error; }
"#,
            ),
        ])
        .output()
        .expect("run timing helper");
    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains("credential-and-prompt"));
    let events: Vec<Value> = stderr
        .lines()
        .map(|line| serde_json::from_str(line.strip_prefix("cursor-sdk-timing ").unwrap()).unwrap())
        .collect();
    assert_eq!(events.len(), 8);
    for pair in events.chunks_exact(2) {
        assert_eq!(pair[0]["phase"], "start");
        assert_eq!(pair[0]["id"], pair[1]["id"]);
        assert_eq!(pair[0]["pid"], pair[1]["pid"]);
        assert!(pair[1]["elapsedMs"].as_f64().unwrap() >= pair[0]["elapsedMs"].as_f64().unwrap());
    }
    assert_eq!(events[1]["phase"], "ok");
    assert!(events[1]["elapsedMs"].as_f64().unwrap() >= 20.0);
    assert_eq!(events[3]["phase"], "error");
    assert_eq!(events[5]["phase"], "ok");
    assert_eq!(events[7]["phase"], "error");
}
