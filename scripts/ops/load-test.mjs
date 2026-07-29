const base = process.env.LUDO_LOAD_URL ?? "http://127.0.0.1:8080";
const concurrency = Number(process.env.LUDO_LOAD_CONCURRENCY ?? 50);
const iterations = Number(process.env.LUDO_LOAD_ITERATIONS ?? 20);
const samples = [];
let failures = 0;

await Promise.all(Array.from({ length: concurrency }, async () => {
  for (let index = 0; index < iterations; index += 1) {
    const started = performance.now();
    const response = await fetch(`${base}/health/ready`).catch(() => null);
    samples.push(performance.now() - started);
    if (!response?.ok) failures += 1;
  }
}));
samples.sort((a, b) => a - b);
const percentile = value => samples[Math.min(samples.length - 1, Math.floor(samples.length * value))];
console.log(JSON.stringify({
  requests: samples.length, failures,
  p50_ms: percentile(.5), p95_ms: percentile(.95), p99_ms: percentile(.99)
}, null, 2));
if (failures || percentile(.95) > Number(process.env.LUDO_LOAD_P95_BUDGET_MS ?? 250)) process.exitCode = 1;
