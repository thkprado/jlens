// CodeMirror 6 integration of the jlens language service: highlighting, completion, diagnostics,
// hover and signature help. Copy this module into an application to get the same editor.
//
// CodeMirror positions and jlens offsets are both UTF-16 code units, so they are used as is.

import { autocompletion } from "@codemirror/autocomplete";
import { HighlightStyle, StreamLanguage, syntaxHighlighting } from "@codemirror/language";
import { linter } from "@codemirror/lint";
import { EditorView, hoverTooltip } from "@codemirror/view";
import { tags } from "@lezer/highlight";

/** A small tokenizer for highlighting JSONata (the language service does the real parsing). */
export const jsonata = StreamLanguage.define({
  name: "jsonata",
  startState: () => ({ comment: false }),
  token(stream, state) {
    if (state.comment || stream.match("/*")) {
      state.comment = true;
      while (!stream.eol()) {
        if (stream.match("*/")) {
          state.comment = false;
          break;
        }
        stream.next();
      }
      return "comment";
    }
    if (stream.eatSpace()) return null;
    const quote = stream.peek();
    if (quote === '"' || quote === "'") {
      stream.next();
      let escaped = false;
      while (!stream.eol()) {
        const c = stream.next();
        if (c === quote && !escaped) break;
        escaped = !escaped && c === "\\";
      }
      return "string";
    }
    if (stream.match(/^`[^`]*`?/)) return "propertyName";
    if (stream.match(/^\d+(\.\d+)?([eE][+-]?\d+)?/)) return "number";
    if (stream.match(/^\$[\p{L}\p{N}_$]*/u)) return "variableName";
    const word = stream.match(/^[\p{L}_][\p{L}\p{N}_]*/u);
    if (word) {
      if (["and", "or", "in", "function"].includes(word[0])) return "keyword";
      if (["true", "false", "null"].includes(word[0])) return "atom";
      return "propertyName";
    }
    stream.next();
    return "operator";
  },
  languageData: { closeBrackets: { brackets: ["(", "[", "{", "'", '"', "`"] } },
});

const highlightStyle = HighlightStyle.define([
  { tag: tags.comment, color: "var(--syntax-comment)", fontStyle: "italic" },
  { tag: tags.string, color: "var(--syntax-string)" },
  { tag: tags.number, color: "var(--syntax-number)" },
  { tag: tags.atom, color: "var(--syntax-number)" },
  { tag: tags.keyword, color: "var(--syntax-keyword)", fontWeight: "600" },
  { tag: tags.variableName, color: "var(--syntax-variable)" },
  { tag: tags.propertyName, color: "var(--syntax-field)" },
  { tag: tags.operator, color: "var(--syntax-operator)" },
]);

const completionTypes = {
  field: "property",
  variable: "variable",
  function: "function",
  keyword: "keyword",
  operator: "keyword",
};

/** Renders the Markdown subset the language service produces (bold, code, paragraphs). */
export function markdown(text) {
  const div = document.createElement("div");
  div.className = "md";
  for (const paragraph of text.split(/\n\n+/)) {
    const p = document.createElement("p");
    // Split on `code` and **bold**, keeping the delimiters.
    for (const part of paragraph.split(/(`[^`]*`|\*\*[^*]+\*\*)/)) {
      if (part.startsWith("`") && part.endsWith("`") && part.length > 1) {
        const code = document.createElement("code");
        code.textContent = part.slice(1, -1);
        p.append(code);
      } else if (part.startsWith("**") && part.endsWith("**") && part.length > 3) {
        const strong = document.createElement("strong");
        strong.textContent = part.slice(2, -2);
        p.append(strong);
      } else {
        p.append(part);
      }
    }
    div.append(p);
  }
  return div;
}

function complete(getService) {
  return (context) => {
    const text = context.state.doc.toString();
    const before = text.slice(0, context.pos);
    const { range, items } = getService().complete(text, context.pos);
    // While typing, only open the list on a word or right after a step, filter or call.
    const triggered = range.start < range.end || /[.[({$,]\s*$/.test(before);
    if (!items.length || (!context.explicit && !triggered)) return null;
    return {
      from: range.start,
      to: range.end,
      filter: false,
      options: items.map((item, rank) => ({
        label: item.label,
        type: completionTypes[item.kind],
        detail: item.detail,
        info: item.documentation ? () => markdown(item.documentation) : undefined,
        apply: item.insertText,
        boost: -rank,
      })),
    };
  };
}

function diagnostics(getService) {
  return (view) => {
    const text = view.state.doc.toString();
    return getService()
      .diagnostics(text)
      .map((d) => ({
        from: Math.min(d.span.start, text.length),
        to: Math.min(Math.max(d.span.end, d.span.start), text.length),
        severity: d.severity,
        source: d.code,
        message: d.message,
      }));
  };
}

function hover(getService) {
  return hoverTooltip((view, pos) => {
    const info = getService().hover(view.state.doc.toString(), pos);
    if (!info) return null;
    return {
      pos: info.range.start,
      end: info.range.end,
      above: true,
      create: () => ({ dom: markdown(info.contents) }),
    };
  });
}

/** Shows the signature of the function call around the cursor in `element`. */
function signatureHelp(getService, element) {
  return EditorView.updateListener.of((update) => {
    if (!update.docChanged && !update.selectionSet && !update.focusChanged) return;
    const state = update.state;
    const help = getService().signatureHelp(state.doc.toString(), state.selection.main.head);
    element.replaceChildren();
    if (!help) return;
    element.append(`${help.name}(`);
    help.params.forEach((param, i) => {
      if (i > 0) element.append(", ");
      const span = document.createElement(i === help.activeParam ? "strong" : "span");
      span.textContent = param;
      element.append(span);
    });
    element.append(`) — ${help.summary}`);
  });
}

/**
 * The editor extensions for JSONata. `getService` returns the current jlens `LanguageService`
 * (it can change, for example when the input schema changes); `signatureElement` receives the
 * signature help.
 */
export function jlensExtensions(getService, signatureElement) {
  return [
    jsonata,
    syntaxHighlighting(highlightStyle),
    autocompletion({ override: [complete(getService)], icons: true }),
    linter(diagnostics(getService), { delay: 150 }),
    hover(getService),
    signatureHelp(getService, signatureElement),
  ];
}
