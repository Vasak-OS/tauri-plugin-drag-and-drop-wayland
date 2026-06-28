import { invoke, Channel } from "@tauri-apps/api/core";

export type DragItem = string[] | {
  data: string | Record<string, string>;
  types: string[];
};

export type DragResult = "Dropped" | "Cancelled";

export interface CursorPosition {
  x: number;
  y: number;
}

export interface DragOptions {
  mode?: "copy" | "move";
}

export interface CallbackPayload {
  result: DragResult;
  cursorPos: CursorPosition;
}

export async function startDrag(
  item: DragItem,
  icon?: string,
  options?: DragOptions,
  onEvent?: (payload: CallbackPayload) => void,
): Promise<void> {
  const onEventChannel = new Channel<CallbackPayload>();
  if (onEvent) {
    onEventChannel.onmessage = onEvent;
  }
  await invoke("plugin:drag-and-drop-wayland|start_drag", {
    item,
    image: icon,
    options: options ?? null,
    onEvent: onEventChannel,
  });
}
