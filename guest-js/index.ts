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
  /** La acción que negoció el destino. Informativa. */
  action: "copy" | "move" | null;
  /**
   * Si el destino pidió que el origen borre lo que entregó.
   *
   * **Esta es la señal que autoriza borrar, y no `action`.** Un destino puede
   * elegir mover y después no pedir el borrado —porque falló al guardar, o porque
   * cambió de idea— y borrar por haber visto «move» sería perder un archivo que
   * nadie copió a ninguna parte.
   */
  sourceShouldDelete: boolean;
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
