/* Types for elo-clip-1.js: `window.EloClip`, clips made in the player's
   browser with WebCodecs. */

export type ClipShape = "tall" | "wide" | "square" | "same";

/** What goes over the picture. */
export interface ClipDress {
  /** "tall" 1080×1920 (default), "wide" 1920×1080, "square" 1080×1080, "same" the canvas at most 1920 on a side. */
  shape?: ClipShape;
  /** A few words across the top for the first three seconds. */
  line?: string;
  /** A word in the corner while it plays. */
  tag?: string;
  /** The end card's title, the line under it and the link it shows. */
  title?: string;
  sub?: string;
  link?: string;
  /** false: no end card. */
  card?: boolean;
  /** A CSS font family, loaded before you save. */
  font?: string;
}

export interface KeepOptions extends ClipDress {
  /** How far back a save reaches, 3 to 60 seconds (30). */
  seconds?: number;
  /** Frames a second kept, 10 to 60 (30). */
  fps?: number;
  /** The node everything you play goes through, or a MediaStream. */
  audio?: AudioNode | MediaStream;
  /** For a MediaStream: the AudioContext to read it with. */
  context?: AudioContext;
  /** Call keep.frame() yourself after each draw instead of on each animation frame. */
  manual?: boolean;
}

export interface Keep {
  /** Encode the canvas now (with `manual: true`, after each draw). */
  frame(): void;
  /** Seconds a save can reach back over now. */
  held(): number;
  /** The last `seconds` (all that is kept, by default) as an MP4. */
  save(options?: ClipDress & { seconds?: number }): Promise<Blob>;
  stop(): void;
}

export interface FilmOptions extends ClipDress {
  /** The video's size; the default is the shape's. */
  width?: number;
  height?: number;
  fps?: number;
}

export interface Film {
  readonly width: number;
  readonly height: number;
  /** A picture of any size, cropped to fill; resolves when it may take the next. */
  frame(source: CanvasImageSource | VideoFrame | ImageData): Promise<void>;
  /** Interleaved floats, appended in order. */
  audio(samples: Float32Array, rate?: number, channels?: number): void;
  done(): Promise<Blob>;
}

export interface EloClipApi {
  readonly version: 1;
  supported(): { video: boolean; audio: boolean; keep: boolean };
  keep(canvas: HTMLCanvasElement | OffscreenCanvas, options?: KeepOptions): Keep;
  film(options?: FilmOptions): Film;
  /** The share sheet where the browser has one for files, else a download. Call from a click. */
  share(video: Blob, options?: { name?: string; title?: string }): Promise<"shared" | "saved" | "cancelled">;
}

declare global {
  interface Window {
    EloClip: EloClipApi;
  }
}
