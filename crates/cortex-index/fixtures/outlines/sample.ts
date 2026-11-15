import { readFile } from "fs";

/** The largest page. */
export const MAX_PAGE = 100;

const notExported = 5;

/**
 * Loads a page.
 * @param id the page
 */
export async function loadPage(id: string): Promise<Page> {
  const inner = () => { return "function nope() {}"; };
  return { id };
}

export const handler = async (event: Event) => {
  return event;
};

const add = (a: number, b: number): number => a + b;

export interface Page {
  id: string;
  render(): string;
  title?: string;
}

export type PageId = string;

export enum Color {
  Red,
  Green,
}

export default class Renderer<T> extends Base implements Drawable {
  private cache = new Map<string, T>();
  static count = 0;

  constructor(private readonly root: Element) {
    super();
  }

  /** Draws everything. */
  async draw(page: Page): Promise<void> {
    const template = `
      function fake() {
    `;
  }

  get size(): number {
    return 1;
  }

  #secret() {}

  onClick = (event: MouseEvent) => {
    this.draw(event as any);
  };
}

namespace Shapes {
  export function area(): number {
    return 0;
  }
}

function* ids() {
  yield 1;
}

module.exports.legacy = function () {};
