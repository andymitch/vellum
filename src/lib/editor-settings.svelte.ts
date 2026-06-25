// Editor input-assist preferences (autocomplete, autocapitalize, autocorrect,
// spellcheck, autoclose brackets). Persisted to localStorage and applied to the
// CodeMirror editor live via Compartments. Defaults are markdown-oriented:
// autocorrect/autocapitalize off (they garble markdown), spellcheck + autoclose on.

export type EditorSettings = {
  autocomplete: boolean;
  autocapitalize: boolean;
  autocorrect: boolean;
  spellcheck: boolean;
  closeBrackets: boolean;
};

const KEY = "notes-editor";

const DEFAULTS: EditorSettings = {
  autocomplete: false,
  autocapitalize: false,
  autocorrect: false,
  spellcheck: true,
  closeBrackets: true,
};

let saved: Partial<EditorSettings> = {};
try {
  saved = JSON.parse(localStorage.getItem(KEY) || "{}");
} catch {
  /* ignore malformed */
}

const state = $state<EditorSettings>({ ...DEFAULTS, ...saved });

function persist() {
  localStorage.setItem(KEY, JSON.stringify(state));
}

// The HTML attributes the editor's content DOM should carry. Booleans map to
// the on/off string values the browser/WKWebView expects.
export function contentAttrs(): Record<string, string> {
  return {
    autocomplete: state.autocomplete ? "on" : "off",
    autocapitalize: state.autocapitalize ? "sentences" : "off",
    autocorrect: state.autocorrect ? "on" : "off",
    spellcheck: String(state.spellcheck),
  };
}

export const editorSettings = {
  get autocomplete() {
    return state.autocomplete;
  },
  set autocomplete(v: boolean) {
    state.autocomplete = v;
    persist();
  },
  get autocapitalize() {
    return state.autocapitalize;
  },
  set autocapitalize(v: boolean) {
    state.autocapitalize = v;
    persist();
  },
  get autocorrect() {
    return state.autocorrect;
  },
  set autocorrect(v: boolean) {
    state.autocorrect = v;
    persist();
  },
  get spellcheck() {
    return state.spellcheck;
  },
  set spellcheck(v: boolean) {
    state.spellcheck = v;
    persist();
  },
  get closeBrackets() {
    return state.closeBrackets;
  },
  set closeBrackets(v: boolean) {
    state.closeBrackets = v;
    persist();
  },
};
