// The moment the window becomes visible. Motion that must be seen, such as
// the empty state's mark playing once, waits for it.
let resolveShown: () => void = () => {};

export const windowShown = new Promise<void>((resolve) => {
  resolveShown = resolve;
});

export function markWindowShown(): void {
  resolveShown();
}
