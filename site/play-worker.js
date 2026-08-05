/* Playground worker: owns the WebAssembly instance and runs rigid programs.
 *
 * Everything happens here rather than on the main thread so a runaway program
 * (rigid has `while` and no timeout of its own) cannot freeze the page — the
 * page terminates this worker and starts a fresh one.
 */

let wasmExports = null;
const encoder = new TextEncoder();
const decoder = new TextDecoder();

async function boot() {
  const response = await fetch("rigid.wasm");
  if (!response.ok) throw new Error("could not load rigid.wasm (" + response.status + ")");
  const bytes = await response.arrayBuffer();
  const { instance } = await WebAssembly.instantiate(bytes, {});
  wasmExports = instance.exports;
}

const ready = boot().then(
  () => postMessage({ type: "ready" }),
  (e) => postMessage({ type: "boot-error", message: String(e && e.message ? e.message : e) })
);

function callWasm(request) {
  const ex = wasmExports;
  const input = encoder.encode(JSON.stringify(request));
  const inPtr = ex.rigid_alloc(input.length);
  new Uint8Array(ex.memory.buffer, inPtr, input.length).set(input);

  const outPtr = ex.rigid_eval(inPtr, input.length);
  // Read length and payload separately: any wasm call can grow memory and
  // detach previously-created views, so never cache buffer views across calls.
  const len = new DataView(ex.memory.buffer).getUint32(outPtr, true);
  const text = decoder.decode(new Uint8Array(ex.memory.buffer, outPtr + 4, len));
  ex.rigid_free(outPtr);
  return JSON.parse(text);
}

onmessage = async (event) => {
  await ready;
  if (!wasmExports) return; // boot failed; the page already knows
  const { id, request } = event.data;
  const started = Date.now();
  try {
    const result = callWasm(request);
    postMessage({ type: "result", id, result, ms: Date.now() - started });
  } catch (e) {
    postMessage({
      type: "result",
      id,
      result: {
        ok: false,
        output: "",
        result: "playground error: " + (e && e.message ? e.message : String(e)),
        diagnostics: [],
      },
      ms: Date.now() - started,
    });
  }
};
