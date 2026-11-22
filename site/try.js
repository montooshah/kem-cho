import { pipeline, env } from 'https://cdn.jsdelivr.net/npm/@huggingface/transformers@4.3.2';

env.allowLocalModels = false;

const MODEL = 'onnx-community/whisper-tiny.en';
const $ = (id) => document.getElementById(id);
const rec = $('rec');
const status = $('status');
const out = $('out');
const level = $('level');

let transcriber;
let recording = null;
let held = false;

function say(text) {
  status.textContent = text;
}

async function pickDevice() {
  try {
    const adapter = await navigator.gpu?.requestAdapter();
    if (adapter && !adapter.info?.isFallbackAdapter) return 'webgpu';
  } catch {}
  return 'wasm';
}

async function load() {
  const device = await pickDevice();
  const progress = {};
  const options = {
    device,
    dtype: device === 'webgpu' ? 'fp32' : 'q8',
    progress_callback: (p) => {
      if (p.status !== 'progress' || !p.total) return;
      progress[p.file] = [p.loaded, p.total];
      const [done, total] = Object.values(progress).reduce((a, b) => [a[0] + b[0], a[1] + b[1]], [0, 0]);
      say(`Downloading model ${Math.round((done / total) * 100)}%`);
    },
  };
  say('Downloading model…');
  transcriber = await pipeline('automatic-speech-recognition', MODEL, options);
  rec.disabled = false;
  rec.textContent = 'Hold to talk';
  say(`Ready (${device === 'webgpu' ? 'GPU' : 'CPU'})`);
}

async function start() {
  if (held || recording || rec.disabled) return;
  held = true;
  let stream;
  try {
    stream = await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true } });
  } catch (e) {
    held = false;
    return say(`Microphone unavailable: ${e.message}`);
  }
  const ctx = new AudioContext({ sampleRate: 16000 });
  const source = ctx.createMediaStreamSource(stream);
  const node = ctx.createScriptProcessor(4096, 1, 1);
  const chunks = [];
  node.onaudioprocess = (e) => {
    const data = e.inputBuffer.getChannelData(0);
    chunks.push(new Float32Array(data));
    let peak = 0;
    for (const v of data) peak = Math.max(peak, Math.abs(v));
    level.style.width = `${Math.min(100, peak * 300)}%`;
  };
  source.connect(node);
  node.connect(ctx.destination);
  recording = { stream, ctx, node, chunks };
  rec.textContent = 'Listening… let go to finish';
  say('');
  if (!held) stop();
}

async function stop() {
  held = false;
  if (!recording) return;
  const { stream, ctx, node, chunks } = recording;
  recording = null;
  node.disconnect();
  stream.getTracks().forEach((t) => t.stop());
  await ctx.close();
  level.style.width = '0';
  rec.textContent = 'Hold to talk';
  const audio = new Float32Array(chunks.reduce((n, c) => n + c.length, 0));
  let offset = 0;
  for (const c of chunks) {
    audio.set(c, offset);
    offset += c.length;
  }
  if (audio.length < 16000 * 0.3) return say('Too short. Hold while you speak.');
  say('Transcribing…');
  rec.disabled = true;
  const started = performance.now();
  try {
    const { text } = await transcriber(audio, { chunk_length_s: 30 });
    const clean = cleanup(text);
    if (clean) out.value = out.value ? `${out.value} ${clean}` : clean;
    say(clean ? `Done in ${((performance.now() - started) / 1000).toFixed(1)} s` : 'No speech heard.');
  } catch (e) {
    say(`Couldn't transcribe: ${e.message}`);
  } finally {
    rec.disabled = false;
  }
}

function cleanup(text) {
  return text
    .replace(/\[[^\]]*\]|\([^)]*\)/g, ' ')
    .replace(/\b(um+|uh+|erm+|hmm+)\b[,.]?\s*/gi, '')
    .replace(/\s+/g, ' ')
    .trim();
}

rec.addEventListener('pointerdown', start);
rec.addEventListener('pointerup', stop);
rec.addEventListener('pointerleave', stop);
document.addEventListener('keydown', (e) => {
  if (e.code === 'Space' && document.activeElement !== out && !e.repeat) {
    e.preventDefault();
    start();
  }
});
document.addEventListener('keyup', (e) => {
  if (e.code === 'Space' && document.activeElement !== out) stop();
});
$('copy').addEventListener('click', () => navigator.clipboard.writeText(out.value));

load().catch((e) => say(`Couldn't load the model: ${e.message}`));
