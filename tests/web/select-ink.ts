// What the Version `<select>` draws, measured from its pixels rather than its computed style: a
// browser can ignore `padding-right` or `text-overflow` on a native control and still report them.
// Shared by tests/web/document.spec.ts and verify/scenarios/issue-5-version-chevron.spec.ts.
import type { Locator } from '@playwright/test';

export interface SelectInk {
  /** The label's width as text, in px, against the room the select's padding leaves for it. */
  label: number;
  room: number;
  width: number;
  /** Background-only pixel columns between the end of the label's ink and the chevron's. */
  gap: number;
}

export async function selectInk(sel: Locator): Promise<SelectInk> {
  const png = (await sel.screenshot()).toString('base64');
  return sel.evaluate(async (e: HTMLSelectElement, b64: string) => {
    const cs = getComputedStyle(e);
    const measure = document.createElement('canvas').getContext('2d')!;
    measure.font = cs.font;
    const blob = await (await fetch(`data:image/png;base64,${b64}`)).blob();
    const img = await createImageBitmap(blob);
    const canvas = document.createElement('canvas');
    canvas.width = img.width;
    canvas.height = img.height;
    const ctx = canvas.getContext('2d')!;
    ctx.drawImage(img, 0, 0);
    const { data } = ctx.getImageData(0, 0, img.width, img.height);
    const px = (x: number, y: number) => [0, 1, 2].map((c) => data[(y * img.width + x) * 4 + c]);
    // The select's own borders are its outermost columns; its background is what is just inside.
    const bg = px(2, 1);
    const inked = (x: number) => {
      for (let y = 1; y < img.height - 1; y++) {
        const p = px(x, y);
        if (Math.max(Math.abs(p[0] - bg[0]), Math.abs(p[1] - bg[1]), Math.abs(p[2] - bg[2])) > 48) return true;
      }
      return false;
    };
    // Reading from the right: the chevron's ink, the blank run before it, then the label's ink.
    let x = img.width - 3;
    while (x > 1 && !inked(x)) x--;
    while (x > 1 && inked(x)) x--;
    let gap = 0;
    while (x > 1 && !inked(x)) { gap++; x--; }
    return {
      label: measure.measureText(e.options[e.selectedIndex].text).width,
      room: e.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight),
      width: e.clientWidth,
      gap: x > 1 ? gap : 0,
    };
  }, png);
}

/** The select's narrower max width (`@container cd (max-width: 1099.98px)` in app.css) applies below this. */
export const SELECT_NARROWS_BELOW = 1100;

/** At least this many blank columns separate a label from the chevron: a word space is about 4. */
export const MIN_INK_GAP = 8;
