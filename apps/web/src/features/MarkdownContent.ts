import { cloneVNode, defineComponent, h, reactive, type VNodeChild } from "vue";
import {
  markdownBlocksWithLines,
  markdownInline,
  safeWebUrl,
  workspaceImagePath,
  type MarkdownInline,
} from "./markdown-model";
import { fenceLanguage, loadHighlighter, type Highlighter } from "./highlighting";
import { hasInlineHtml, parseHtml, type HtmlNode } from "./html-subset";
import { openLightbox } from "./image-lightbox";
import { mediaKind } from "./media-kind";
import CopyButton from "./CopyButton.vue";

/**
 * Renders the markdown subset both the conversation and the file pane show.
 *
 * Shared rather than copied so a file and a message that contain the same text
 * render the same way — and so a fix to either reaches both. The `chat-`
 * classes are kept as the style contract; the file pane scopes its own spacing
 * around them instead of restyling each element.
 */
export const MarkdownContent = defineComponent({
  props: {
    text: { type: String, required: true },
    /** Turns a workspace path into a URL to load it from. Without one, an
     * image stays as its alt text: nothing is loaded that was not asked for. */
    imageUrl: { type: Function as unknown as () => (path: string) => string, required: false },
    /** Turns an absolute host path into a URL to load it from. Without one, a
     * leading slash means the workspace root, as it does in a repository's docs;
     * with one -- in a conversation, where agents write real paths -- it means
     * the host's file system. */
    hostImageUrl: { type: Function as unknown as () => (path: string) => string, required: false },
    /** Directory of the file being rendered, for relative image paths. */
    base: { type: String, default: "" },
    /** Opens a file a message points at. Without one, references render as
     * the text they were written as. */
    openFile: {
      type: Function as unknown as () => (reference: { path: string; line?: number }) => void,
      required: false,
    },
    /** Marks each block with the source line it starts on (`data-line`), so an
     * editor beside it can keep the two aligned. */
    sourceLines: { type: Boolean, default: false },
    /** Reads HTML blocks as the elements html-subset.ts allows, as a
     * repository's README is written. A message shows its `<` as text. */
    html: { type: Boolean, default: false },
  },
  setup(props) {
    /** Images that would not load: shown as a link to the file instead, which
     * can explain why -- outside the folders Tendly may browse, say. */
    const failed = reactive(new Set<string>());
    const hostPath = (src: string) => {
      let value = src.trim().replace(/^file:\/\//i, "");
      try {
        value = decodeURIComponent(value);
      } catch {
        return undefined;
      }
      value = value.split(/[?#]/)[0];
      return /^(\/(?!\/)|[A-Za-z]:[\\/])/.test(value) && ["image", "converted"].includes(mediaKind(value))
        ? value
        : undefined;
    };
    const imageSource = (src: string): { url: string; path: string } | undefined => {
      const absolute = props.hostImageUrl && hostPath(src);
      if (absolute) return { url: props.hostImageUrl!(absolute), path: absolute };
      const path = props.imageUrl && workspaceImagePath(src, props.base);
      return path ? { url: props.imageUrl!(path), path } : undefined;
    };
    /** Grammars for the code blocks on screen, fetched the first time one names them. */
    const grammars = reactive(new Map<string, Highlighter | null>());
    const highlighted = (code: string, name: string): string | undefined => {
      const language = fenceLanguage(name);
      if (!language || code.length > 64 * 1024) return undefined;
      if (!grammars.has(language)) {
        grammars.set(language, null);
        void loadHighlighter(language)
          .then(loaded => {
            grammars.set(language, loaded ?? null);
          })
          .catch(() => {});
      }
      const highlight = grammars.get(language);
      return highlight ? highlight(code, language) : undefined;
    };
    const image = (src: string, alt: string, extra: Record<string, unknown> = {}) => {
      const found = imageSource(src);
      if (!found) return undefined;
      const { path } = found;
      const url =
        mediaKind(path) === "converted" ? `${found.url}${found.url.includes("?") ? "&" : "?"}as=png` : found.url;
      if (failed.has(url)) return { failed: true as const, path, url };
      return {
        failed: false as const,
        path,
        url,
        node: h("img", {
          class: "chat-inline-image",
          src: url,
          alt,
          title: alt || path,
          loading: "lazy",
          onClick: () => openLightbox(url, alt || path),
          onError: () => failed.add(url),
          ...extra,
        }),
      };
    };
    const darkPage = () => {
      if (typeof document === "undefined") return false;
      const theme = document.documentElement.dataset.theme;
      return (
        theme === "dark" || (theme !== "light" && !!globalThis.matchMedia?.("(prefers-color-scheme: dark)").matches)
      );
    };
    const ALIGN = new Set(["left", "center", "right"]);
    /** Vue nodes for the HTML a file wrote: only the tree parseHtml allowed. */
    const htmlNodes = (nodes: HtmlNode[], inLink = false): VNodeChild[] =>
      nodes.map(node => {
        if (typeof node === "string") return node;
        const { tag, attrs, children } = node;
        const align = attrs.align && ALIGN.has(attrs.align.toLowerCase()) ? attrs.align.toLowerCase() : undefined;
        if (tag === "img" || tag === "picture") {
          const img =
            tag === "img"
              ? node
              : children.find(
                  (child): child is Exclude<HtmlNode, string> => typeof child !== "string" && child.tag === "img",
                );
          if (!img) return null;
          // <picture> offers a dark variant through <source media=…dark>; the page's own theme picks it.
          const dark =
            tag === "picture" && darkPage()
              ? children.find(
                  (child): child is Exclude<HtmlNode, string> =>
                    typeof child !== "string" && child.tag === "source" && /dark/i.test(child.attrs.media ?? ""),
                )
              : undefined;
          const src = (dark?.attrs.srcset ?? "").trim().split(/\s+/)[0] || img.attrs.src || "";
          const alt = img.attrs.alt ?? "";
          const size = (value?: string) => (value && /^\d{1,4}%?$/.test(value) ? value : undefined);
          const local = image(src, alt, { width: size(img.attrs.width), height: size(img.attrs.height) });
          if (local && !local.failed) return local.node;
          const href = local ? undefined : safeWebUrl(src);
          const label = h("span", { class: "md-remote-image", title: src }, alt || src.split("/").pop() || src);
          return href && !inLink ? h("a", { href, target: "_blank", rel: "noopener noreferrer" }, [label]) : label;
        }
        if (tag === "source") return null;
        if (tag === "a") {
          const href = attrs.href ? safeWebUrl(attrs.href) : undefined;
          const inner = htmlNodes(children, inLink || !!href);
          return href && !inLink
            ? h("a", { href, title: attrs.title, target: "_blank", rel: "noopener noreferrer" }, inner)
            : h("span", inner);
        }
        if (tag === "br" || tag === "hr") return h(tag);
        if (tag === "center") return h("div", { align: "center" }, htmlNodes(children, inLink));
        if (tag === "td" || tag === "th") {
          const valign = /^(top|middle|bottom)$/i.test(attrs.valign ?? "") ? attrs.valign.toLowerCase() : undefined;
          const width = /^\d{1,4}(%|px)?$/.test(attrs.width ?? "")
            ? /\d$/.test(attrs.width)
              ? attrs.width + "px"
              : attrs.width
            : undefined;
          const span = (value?: string) => (value && /^\d{1,2}$/.test(value) ? Number(value) : undefined);
          return h(
            tag,
            {
              colspan: span(attrs.colspan),
              rowspan: span(attrs.rowspan),
              style: { textAlign: align, verticalAlign: valign, width },
            },
            htmlNodes(children, inLink),
          );
        }
        if (tag === "details") return h("details", { open: "open" in attrs || undefined }, htmlNodes(children, inLink));
        return h(tag, align ? { align } : {}, htmlNodes(children, inLink));
      });
    const inline = (text: string) =>
      markdownInline(text).map((part: MarkdownInline) =>
        part.type === "file"
          ? props.openFile
            ? (() => {
                const slash = part.path.lastIndexOf("/"),
                  name = part.path.slice(slash + 1),
                  dir = slash > 0 ? part.path.slice(0, slash + 1) : "";
                return h(
                  "button",
                  {
                    type: "button",
                    class: ["chat-file-ref", { "is-code": part.code }],
                    title: part.path + (part.line ? `:${part.line}` : ""),
                    onClick: () => props.openFile!({ path: part.path, line: part.line }),
                  },
                  [
                    h("svg", { viewBox: "0 0 16 16", width: 11, height: 11, "aria-hidden": "true" }, [
                      h("path", {
                        d: "M4 1.5h5l3 3v10H4zM9 1.5v3h3",
                        fill: "none",
                        stroke: "currentColor",
                        "stroke-width": 1.3,
                        "stroke-linejoin": "round",
                      }),
                    ]),
                    dir ? h("span", { class: "chat-file-dir" }, dir.length > 28 ? "…" + dir.slice(-27) : dir) : null,
                    h("span", { class: "chat-file-name" }, name),
                    part.line ? h("span", { class: "chat-file-line" }, `:${part.line}`) : null,
                  ],
                );
              })()
            : part.code
              ? h("code", part.text)
              : part.text
          : part.type === "image"
            ? (() => {
                const found = image(part.src, part.alt);
                if (!found) return part.alt || part.src;
                if (found.failed) {
                  const { path } = found;
                  return props.openFile
                    ? h(
                        "button",
                        {
                          type: "button",
                          class: "chat-file-ref",
                          title: path,
                          onClick: () => props.openFile!({ path }),
                        },
                        [h("span", { class: "chat-file-name" }, part.alt || path.split("/").pop())],
                      )
                    : part.alt || part.src;
                }
                return found.node;
              })()
            : part.type === "link"
              ? // Untrusted destinations: opening in a new tab without these lets the
                // opened page reach back through window.opener.
                h("a", { href: part.href, target: "_blank", rel: "noopener noreferrer" }, part.text)
              : part.type === "text"
                ? props.html && hasInlineHtml(part.text)
                  ? htmlNodes(parseHtml(part.text, true))
                  : part.text
                : h(part.type === "strong" ? "strong" : part.type === "em" ? "em" : "code", part.text),
      );
    return () =>
      h(
        "div",
        { class: "chat-markdown" },
        markdownBlocksWithLines(props.text, { html: props.html }).map(({ block, line }) => {
          const node =
            block.type === "code"
              ? (() => {
                  // Highlighter output is escaped source wrapped in spans of its own; nothing the text wrote becomes markup.
                  const colored = highlighted(block.text, block.language);
                  return h("div", { class: "chat-code" }, [
                    block.language ? h("small", block.language) : null,
                    h("div", { class: "chat-code-copy" }, [
                      h(CopyButton, { text: block.text, label: "Copy code", compact: true }),
                    ]),
                    h("pre", [colored === undefined ? h("code", block.text) : h("code", { innerHTML: colored })]),
                  ]);
                })()
              : block.type === "rule"
                ? h("hr")
                : block.type === "html"
                  ? h("div", { class: "md-html" }, htmlNodes(parseHtml(block.text)))
                  : block.type === "table"
                    ? // Wide tables scroll inside their own box rather than the message.
                      h("div", { class: "chat-table" }, [
                        h("table", [
                          h("thead", [
                            h(
                              "tr",
                              block.header.map((cell, index) =>
                                h(
                                  "th",
                                  { style: block.align[index] ? { textAlign: block.align[index] } : undefined },
                                  inline(cell),
                                ),
                              ),
                            ),
                          ]),
                          h(
                            "tbody",
                            block.rows.map(row =>
                              h(
                                "tr",
                                row.map((cell, index) =>
                                  h(
                                    "td",
                                    { style: block.align[index] ? { textAlign: block.align[index] } : undefined },
                                    inline(cell),
                                  ),
                                ),
                              ),
                            ),
                          ),
                        ]),
                      ])
                    : block.type === "list"
                      ? h(
                          block.ordered ? "ol" : "ul",
                          block.items.map(item => {
                            // `- [ ] todo` and `- [x] done`: a box drawn, not an input to tick.
                            const task = item.match(/^\[([ xX])\]\s+(.*)$/s);
                            if (!task) return h("li", inline(item));
                            const done = task[1] !== " ";
                            return h("li", { class: "is-task" }, [
                              h(
                                "span",
                                {
                                  class: ["task-box", { "is-done": done }],
                                  role: "img",
                                  "aria-label": done ? "done" : "to do",
                                },
                                done ? "✓" : "",
                              ),
                              ...inline(task[2]),
                            ]);
                          }),
                        )
                      : block.type === "quote"
                        ? h("blockquote", [h("p", inline(block.text))])
                        : h(block.type === "heading" ? `h${Math.min(block.level ?? 3, 6)}` : "p", inline(block.text));
          return props.sourceLines ? cloneVNode(node, { "data-line": line }) : node;
        }),
      );
  },
});
