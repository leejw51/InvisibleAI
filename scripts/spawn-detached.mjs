// A managed backend or capture client must outlive its launcher's process group.
// Node is already required by make start's frontend build.
import {spawn} from 'node:child_process';
import {openSync, closeSync} from 'node:fs';

const [log, executable, ...args] = process.argv.slice(2);
if (!log || !executable) {
  console.error('Usage: spawn-detached.mjs LOG EXECUTABLE [ARGS...]');
  process.exit(1);
}
const output = openSync(log, 'w', 0o600);
try {
  const child = spawn(executable, args, {detached: true, stdio: ['ignore', output, output]});
  child.once('error', error => {
    console.error(`Could not start managed process: ${error.message}`);
    process.exitCode = 1;
  });
  child.once('spawn', () => {
    // console.log(number) adds ANSI escapes under FORCE_COLOR (e.g. Playwright).
    // The PID file is a machine-readable decimal value in every environment.
    process.stdout.write(`${child.pid}\n`);
    child.unref();
  });
} finally {
  closeSync(output);
}
