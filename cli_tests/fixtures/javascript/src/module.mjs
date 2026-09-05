export async function load(url) {
  return await fetch(url);
}

export function* sequence() {
  yield "first";
  yield "second";
}

export default class {
  static create() {
    return new this();
  }
}
