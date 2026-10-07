// Only fixed operation names and numbers go to the log, never SDK arguments or errors.
let timingSequence = 0;
let timingOutput = (event) => process.stderr.write(`cursor-sdk-timing ${JSON.stringify(event)}\n`);
function beginTiming(operation) {
  const id = ++timingSequence;
  const start = performance.now();
  const record = (phase) => timingOutput({ type: "timing", pid: process.pid, id, operation, phase,
    elapsedMs: Math.round((performance.now() - start) * 1000) / 1000 });
  record("start");
  return record;
}
async function timed(operation, call) {
  const record = beginTiming(operation);
  try {
    const result = await call();
    record("ok");
    return result;
  } catch (error) {
    record("error");
    throw error;
  }
}
function timedSync(operation, call) {
  const record = beginTiming(operation);
  try {
    const result = call();
    record("ok");
    return result;
  } catch (error) {
    record("error");
    throw error;
  }
}
