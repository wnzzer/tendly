import test from "node:test";
import assert from "node:assert/strict";
import { extensionOf, mediaKind } from "./media-kind.ts";

test("every format a browser shows is previewed as itself, whatever the case of its extension", () => {
  for (const name of [
    "a.jpg",
    "B.JPG",
    "c.jpeg",
    "d.jfif",
    "e.png",
    "f.apng",
    "g.gif",
    "h.webp",
    "i.avif",
    "j.svg",
    "k.bmp",
    "l.ico",
  ])
    assert.equal(mediaKind(name), "image", name);
  for (const name of ["a.mp4", "b.MOV", "c.webm", "d.mkv", "e.m4v"]) assert.equal(mediaKind(name), "video", name);
  for (const name of ["a.mp3", "b.wav", "c.ogg", "d.opus", "e.m4a", "f.aac", "g.flac"])
    assert.equal(mediaKind(name), "audio", name);
  assert.equal(mediaKind("docs/Spec.PDF"), "pdf");
});

test("images no browser decodes go through the server as PNG; known binaries offer a download, not a text editor", () => {
  for (const name of ["scan.tif", "scan.TIFF", "sprite.tga", "a.ppm", "a.qoi"])
    assert.equal(mediaKind(name), "converted", name);
  for (const name of ["IMG_0001.HEIC", "design.psd", "clip.avi", "bundle.zip", "report.docx", "font.woff2"])
    assert.equal(mediaKind(name), "binary", name);
  for (const name of ["main.rs", "README", ".env", "Makefile", "notes.txt", "dir.jpg/file"])
    assert.equal(mediaKind(name), "text", name);
  assert.equal(extensionOf("/home/me/.hidden"), "");
  assert.equal(extensionOf("C:\\photos\\Trip.JPEG"), "jpeg");
});
