interface MyInterface {
    abstractMethod(): void;
}

class MyClass implements MyInterface {
    constructor(private field: number) {}

    myMethod(): void {
    }

    abstractMethod(): void {
    }
}

function topLevelFunction(): void {
}

type MyType = string | number;
