import { useCallback, useState } from "react";

export interface InspectorDialogs {
  delete: {
    open: boolean;
    confirmText: string;
    setOpen: (open: boolean) => void;
    setConfirmText: (text: string) => void;
    request: () => void;
  };
  rename: {
    open: boolean;
    value: string;
    setOpen: (open: boolean) => void;
    setValue: (value: string) => void;
    request: () => void;
  };
  expire: {
    open: boolean;
    value: string;
    setOpen: (open: boolean) => void;
    setValue: (value: string) => void;
    request: () => void;
  };
}

/**
 * Bundles the open + draft-value state for each of the three modal actions.
 * Each `request()` resets the draft and opens the dialog, matching the
 * inline behaviour the inspector toolbar buttons used to do manually.
 */
export function useInspectorDialogs(keyName: string): InspectorDialogs {
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [deleteConfirm, setDeleteConfirm] = useState("");
  const [renameOpen, setRenameOpen] = useState(false);
  const [renameValue, setRenameValue] = useState("");
  const [expireOpen, setExpireOpen] = useState(false);
  const [expireValue, setExpireValue] = useState("");

  const requestDelete = useCallback(() => {
    setDeleteConfirm("");
    setDeleteOpen(true);
  }, []);

  const requestRename = useCallback(() => {
    setRenameValue(keyName);
    setRenameOpen(true);
  }, [keyName]);

  const requestExpire = useCallback(() => {
    setExpireValue("");
    setExpireOpen(true);
  }, []);

  return {
    delete: {
      open: deleteOpen,
      confirmText: deleteConfirm,
      setOpen: setDeleteOpen,
      setConfirmText: setDeleteConfirm,
      request: requestDelete,
    },
    rename: {
      open: renameOpen,
      value: renameValue,
      setOpen: setRenameOpen,
      setValue: setRenameValue,
      request: requestRename,
    },
    expire: {
      open: expireOpen,
      value: expireValue,
      setOpen: setExpireOpen,
      setValue: setExpireValue,
      request: requestExpire,
    },
  };
}
