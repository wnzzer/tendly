<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import {
  autocompletion,
  closeBrackets,
  closeBracketsKeymap,
  completionKeymap,
  type CompletionSource,
} from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { javascript } from "@codemirror/lang-javascript";
import { bracketMatching, HighlightStyle, indentOnInput, syntaxHighlighting } from "@codemirror/language";
import { Compartment, EditorState } from "@codemirror/state";
import {
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
  placeholder as placeholderText,
} from "@codemirror/view";
import { tags } from "@lezer/highlight";

/**
 * A small JavaScript editor: line numbers, highlighting in the app's own
 * syntax colours (--syn-*, so both themes follow), bracket matching, Tab to
 * indent and undo. `completions` adds names the script can reach, such as ctx.
 */
const props = defineProps<{
  modelValue: string;
  label: string;
  disabled?: boolean;
  placeholder?: string;
  completions?: CompletionSource;
}>();
const emit = defineEmits<{ "update:modelValue": [value: string] }>();
const host = ref<HTMLElement>();
const editable = new Compartment();
let view: EditorView | undefined;

const highlight = HighlightStyle.define([
  { tag: [tags.comment, tags.lineComment, tags.blockComment], color: "var(--syn-comment)", fontStyle: "italic" },
  {
    tag: [tags.keyword, tags.controlKeyword, tags.moduleKeyword, tags.operatorKeyword, tags.definitionKeyword],
    color: "var(--syn-keyword)",
  },
  { tag: [tags.string, tags.special(tags.string), tags.regexp], color: "var(--syn-string)" },
  { tag: [tags.number, tags.bool, tags.null, tags.atom], color: "var(--syn-number)" },
  { tag: [tags.function(tags.variableName), tags.function(tags.propertyName)], color: "var(--syn-title)" },
  { tag: [tags.className, tags.typeName], color: "var(--syn-type)" },
  { tag: [tags.propertyName, tags.attributeName], color: "var(--syn-attr)" },
  { tag: [tags.definition(tags.variableName)], color: "var(--syn-text)" },
  { tag: [tags.punctuation, tags.bracket, tags.operator], color: "var(--syn-meta)" },
]);

const theme = EditorView.theme({
  "&": { color: "var(--syn-text)", backgroundColor: "var(--surface)", fontSize: "var(--text-sm)" },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": { fontFamily: "var(--mono)", lineHeight: "1.65" },
  ".cm-content": { padding: "10px 0", caretColor: "var(--accent)" },
  ".cm-gutters": { backgroundColor: "var(--surface)", color: "var(--code-gutter)", border: "none", paddingLeft: "4px" },
  ".cm-activeLine": { backgroundColor: "var(--code-line)" },
  ".cm-activeLineGutter": { backgroundColor: "var(--code-line)", color: "var(--ink-soft)" },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": {
    backgroundColor: "var(--code-selection) !important",
  },
  ".cm-cursor": { borderLeftColor: "var(--accent)" },
  "&.cm-focused .cm-matchingBracket": {
    backgroundColor: "var(--accent-soft)",
    outline: "1px solid var(--accent-line)",
  },
  ".cm-placeholder": { color: "var(--faint)" },
  ".cm-tooltip": {
    border: "1px solid var(--line)",
    backgroundColor: "var(--surface)",
    borderRadius: "var(--radius-sm)",
    boxShadow: "var(--shadow-pop, 0 6px 20px #0002)",
  },
  ".cm-tooltip-autocomplete ul li[aria-selected]": {
    backgroundColor: "var(--selected, var(--accent-soft))",
    color: "var(--ink)",
  },
  ".cm-completionDetail": { color: "var(--muted)", fontStyle: "normal", marginLeft: "8px" },
});

onMounted(() => {
  view = new EditorView({
    parent: host.value,
    state: EditorState.create({
      doc: props.modelValue,
      extensions: [
        lineNumbers(),
        highlightActiveLineGutter(),
        highlightActiveLine(),
        history(),
        indentOnInput(),
        bracketMatching(),
        closeBrackets(),
        javascript(),
        syntaxHighlighting(highlight),
        theme,
        EditorView.lineWrapping,
        autocompletion({ override: props.completions ? [props.completions] : null, activateOnTyping: true }),
        keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...historyKeymap, ...completionKeymap, indentWithTab]),
        placeholderText(props.placeholder ?? ""),
        EditorView.contentAttributes.of({
          "aria-label": props.label,
          spellcheck: "false",
          autocapitalize: "off",
          autocorrect: "off",
        }),
        editable.of(EditorView.editable.of(!props.disabled)),
        EditorView.updateListener.of(update => {
          if (update.docChanged) emit("update:modelValue", update.state.doc.toString());
        }),
      ],
    }),
  });
});
onBeforeUnmount(() => view?.destroy());

// The parent can replace the text (loading a saved query, picking an example).
watch(
  () => props.modelValue,
  value => {
    if (view && value !== view.state.doc.toString())
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: value } });
  },
);
watch(
  () => props.disabled,
  disabled => view?.dispatch({ effects: editable.reconfigure(EditorView.editable.of(!disabled)) }),
);
</script>

<template>
  <div ref="host" :class="['code-editor', { disabled }]" />
</template>

<style scoped>
.code-editor {
  border: 1px solid var(--line);
  border-radius: var(--radius-md);
  overflow: hidden;
  background: var(--surface);
  transition:
    border-color var(--duration-fast),
    box-shadow var(--duration-fast);
}
.code-editor:focus-within {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px var(--accent-glow);
}
.code-editor.disabled {
  opacity: 0.6;
}
.code-editor :deep(.cm-editor) {
  max-height: 420px;
}
.code-editor :deep(.cm-content),
.code-editor :deep(.cm-gutter) {
  min-height: 170px;
}
.code-editor :deep(.cm-scroller) {
  overflow: auto;
}
</style>
