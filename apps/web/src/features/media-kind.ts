/**
 * How a file is previewed, from its name.
 *
 * `image`, `video`, `audio` and `pdf` are what browsers show themselves.
 * `converted` images are decoded by the server and sent on as PNG (`?as=png`),
 * since no browser reads TIFF, TGA, PNM or QOI. `binary` is a format known not
 * to be text that nothing here can show -- HEIC, PSD, archives, office files --
 * so the pane offers a download instead of failing to read it as text.
 * Everything else is opened as text, which also reports when it is not.
 */
export type MediaKind = "image" | "converted" | "video" | "audio" | "pdf" | "binary" | "text";

const KINDS: Array<[MediaKind, string[]]> = [
  [
    "image",
    ["png", "apng", "jpg", "jpeg", "jpe", "jfif", "pjpeg", "pjp", "gif", "webp", "avif", "svg", "bmp", "ico", "cur"],
  ],
  ["converted", ["tif", "tiff", "tga", "pnm", "pbm", "pgm", "ppm", "qoi"]],
  ["video", ["mp4", "m4v", "webm", "mov", "mkv", "ogv"]],
  ["audio", ["mp3", "wav", "ogg", "oga", "opus", "m4a", "aac", "flac", "weba"]],
  ["pdf", ["pdf"]],
  [
    "binary",
    [
      "heic",
      "heif",
      "psd",
      "ai",
      "raw",
      "cr2",
      "nef",
      "arw",
      "dng",
      "jxl",
      "avi",
      "wmv",
      "flv",
      "3gp",
      "mpg",
      "mpeg",
      "wma",
      "amr",
      "zip",
      "gz",
      "tgz",
      "bz2",
      "xz",
      "zst",
      "7z",
      "rar",
      "tar",
      "jar",
      "war",
      "apk",
      "ipa",
      "dmg",
      "iso",
      "img",
      "exe",
      "dll",
      "so",
      "dylib",
      "bin",
      "o",
      "a",
      "class",
      "wasm",
      "doc",
      "docx",
      "xls",
      "xlsx",
      "ppt",
      "pptx",
      "odt",
      "ods",
      "odp",
      "key",
      "pages",
      "numbers",
      "epub",
      "mobi",
      "ttf",
      "otf",
      "woff",
      "woff2",
      "eot",
      "sqlite",
      "db",
      "pyc",
      "pkl",
      "npy",
      "pt",
      "onnx",
      "safetensors",
    ],
  ],
];
const BY_EXTENSION = new Map(
  KINDS.flatMap(([kind, extensions]) => extensions.map(extension => [extension, kind] as const)),
);

export function extensionOf(path: string): string {
  const name = path.split(/[\\/]/).pop() ?? "";
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
}

export function mediaKind(path: string): MediaKind {
  return BY_EXTENSION.get(extensionOf(path)) ?? "text";
}
