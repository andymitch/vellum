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
  // Mobile only: tapping a previewed note jumps into source view + keyboard,
  // and hiding the keyboard returns to preview (#33).
  quickEdit: boolean;
  // Preview mode renders a card for a link that sits alone on its line (#62).
  // Cards for `[[note]]` links are built locally; cards for http(s) links need
  // the page's Open Graph tags, which means a request to that site — the only
  // outbound traffic Vellum makes. On by default (an invisible feature is no
  // feature), off in one tap for anyone who would rather not reach out.
  linkPreviews: boolean;
};

const KEY = "vellum-editor";

const DEFAULTS: EditorSettings = {
  autocomplete: false,
  autocapitalize: false,
  autocorrect: false,
  spellcheck: true,
  closeBrackets: true,
  quickEdit: false,
  linkPreviews: true,
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
  get quickEdit() {
    return state.quickEdit;
  },
  set quickEdit(v: boolean) {
    state.quickEdit = v;
    persist();
  },
  get linkPreviews() {
    return state.linkPreviews;
  },
  set linkPreviews(v: boolean) {
    state.linkPreviews = v;
    persist();
  },
};
