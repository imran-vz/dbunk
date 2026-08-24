export type MonacoPosition = {
  lineNumber: number;
  column: number;
};

export type MonacoTextModel = {
  getValue: () => string;
  getWordUntilPosition: (position: MonacoPosition) => {
    startColumn: number;
    endColumn: number;
  };
  // oxlint-disable-next-line anti-slop/no-unknown-parameters -- The value is handled at a typed library or domain boundary here.
  getValueInRange: (range: unknown) => string;
  /** Caret clamping (Plan 010) — present on real models; optional so
   *  test doubles stay minimal. */
  getLineCount?: () => number;
  getLineMaxColumn?: (lineNumber: number) => number;
};

/** Monaco `Selection` members the caret persistence reads/writes. */
export type MonacoSelectionRange = {
  selectionStartLineNumber: number;
  selectionStartColumn: number;
  positionLineNumber: number;
  positionColumn: number;
};

export type MonacoCompletionDisposable = {
  dispose: () => void;
};

export type MonacoMouseEvent = {
  target: {
    type: number;
    position?: MonacoPosition | null;
  };
  event?: {
    preventDefault?: () => void;
  };
};

export type MonacoEditorInstance = {
  getPosition: () => MonacoPosition | null;
  // oxlint-disable-next-line anti-slop/no-unknown-returns -- The value is handled at a typed library or domain boundary here.
  getSelection: () => unknown;
  getModel: () => MonacoTextModel | null;
  addAction?: (descriptor: {
    id: string;
    label: string;
    keybindings?: number[];
    contextMenuGroupId?: string;
    contextMenuOrder?: number;
    run: () => void;
  }) => MonacoCompletionDisposable;
  createDecorationsCollection?: (decorations?: unknown[]) => {
    set: (decorations: unknown[]) => void;
    clear: () => void;
  };
  onMouseDown?: (
    listener: (event: MonacoMouseEvent) => void,
  ) => MonacoCompletionDisposable;
  onDidChangeModelContent?: (
    listener: () => void,
  ) => MonacoCompletionDisposable;
  onDidChangeCursorPosition?: (
    listener: (event: { position: MonacoPosition }) => void,
  ) => MonacoCompletionDisposable;
  setSelection?: (range: {
    startLineNumber: number;
    startColumn: number;
    endLineNumber: number;
    endColumn: number;
  }) => void;
  revealPositionInCenterIfOutsideViewport?: (
    position: MonacoPosition,
  ) => void;
};
