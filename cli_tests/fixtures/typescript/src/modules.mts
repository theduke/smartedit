/** Load an ESM module. */
export async function load(url: URL): Promise<string> {
  return (await fetch(url)).text();
}

export default class ModuleLoader {
  static create(): ModuleLoader {
    return new ModuleLoader();
  }
}
