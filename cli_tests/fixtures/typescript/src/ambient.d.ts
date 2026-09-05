declare namespace Fixtures {
  interface Options {
    verbose?: boolean;
  }

  namespace Internal {
    export function trace(message: string): void;
  }
}

declare module "virtual:service" {
  export interface Plugin {
    install(options?: Fixtures.Options): void;
  }
  export const plugin: Plugin;
}

declare global {
  interface Window {
    fixtureService?: Fixtures.Internal;
  }
}

export {};
