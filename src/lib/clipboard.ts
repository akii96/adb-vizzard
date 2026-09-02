/**
 * Clipboard write with a fallback.
 *
 * The webview is a secure context, so the async clipboard API is normally
 * available. It still fails when the window is not focused, which happens if a
 * native dialog is closing, so a hidden textarea and `execCommand` cover that
 * rather than losing the copy.
 */
export async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    return;
  } catch {
    if (!legacyCopy(text)) {
      throw new Error("Could not write to the clipboard.");
    }
  }
}

function legacyCopy(text: string): boolean {
  const area = document.createElement("textarea");
  area.value = text;
  // Off-screen rather than hidden: `display: none` is not selectable.
  area.style.position = "fixed";
  area.style.top = "-1000px";
  area.setAttribute("readonly", "");

  document.body.appendChild(area);
  area.select();

  try {
    return document.execCommand("copy");
  } catch {
    return false;
  } finally {
    document.body.removeChild(area);
  }
}
