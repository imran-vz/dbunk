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
  getValueInRange: (range: unknown) => string;
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
};
