/*
 * Live audio for the /play page: the microphone, or an audio file the
 * visitor picks, mixed to mono and handed to the engine's analyser — the
 * same strobe-audio DSP the app runs on a capture device (D524).
 *
 * An AudioWorklet copies each 128-sample block to the page, which batches
 * them and pushes once per frame. A file also plays out of the speakers; the
 * microphone does not, so it cannot feed back.
 */

const WORKLET = `
class StrobeTap extends AudioWorkletProcessor {
  process(inputs) {
    const input = inputs[0];
    if (input && input.length > 0) {
      const frames = input[0].length;
      const mono = new Float32Array(frames);
      for (const channel of input) {
        for (let i = 0; i < frames; i += 1) mono[i] += channel[i] / input.length;
      }
      this.port.postMessage(mono, [mono.buffer]);
    }
    return true;
  }
}
registerProcessor("strobe-tap", StrobeTap);
`;

export type LiveAudio = {
  sampleRate: number;
  /** The samples gathered since the last call, oldest first. */
  drain(): Float32Array;
  /** The element playing a file, for its play/pause controls. */
  element?: HTMLAudioElement;
  stop(): void;
};

/** Most samples kept between frames: a second, if the page stalls. */
const MAX_PENDING = 48_000;

async function tap(
  context: AudioContext,
  source: AudioNode,
  audible: boolean,
): Promise<Omit<LiveAudio, "stop" | "element">> {
  const url = URL.createObjectURL(new Blob([WORKLET], { type: "text/javascript" }));
  try {
    await context.audioWorklet.addModule(url);
  } finally {
    URL.revokeObjectURL(url);
  }
  const node = new AudioWorkletNode(context, "strobe-tap", { numberOfOutputs: 0 });
  let pending: Float32Array[] = [];
  let length = 0;
  node.port.onmessage = (event: MessageEvent<Float32Array>) => {
    pending.push(event.data);
    length += event.data.length;
    while (length > MAX_PENDING && pending.length > 1) {
      length -= pending.shift()?.length ?? 0;
    }
  };
  source.connect(node);
  if (audible) source.connect(context.destination);
  return {
    sampleRate: context.sampleRate,
    drain() {
      const out = new Float32Array(length);
      let at = 0;
      for (const block of pending) {
        out.set(block, at);
        at += block.length;
      }
      pending = [];
      length = 0;
      return out;
    },
  };
}

/** The microphone. Throws if the visitor refuses it or there is none. */
export async function openMicrophone(): Promise<LiveAudio> {
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: { echoCancellation: false, noiseSuppression: false, autoGainControl: false },
  });
  const context = new AudioContext();
  const live = await tap(context, context.createMediaStreamSource(stream), false);
  return {
    ...live,
    stop() {
      for (const track of stream.getTracks()) track.stop();
      void context.close();
    },
  };
}

/** An audio file, played on a loop. */
export async function openFile(file: File): Promise<LiveAudio> {
  const element = new Audio(URL.createObjectURL(file));
  element.loop = true;
  const context = new AudioContext();
  const live = await tap(context, context.createMediaElementSource(element), true);
  await element.play();
  return {
    ...live,
    element,
    stop() {
      element.pause();
      URL.revokeObjectURL(element.src);
      void context.close();
    },
  };
}
