package com.example;

/**
 * A sample class
 */
public class MyClass {
    private int field;

    public MyClass(int field) {
        this.field = field;
    }

    public void myMethod(String arg) {
        System.out.println(arg);
    }
}

interface MyInterface {
    void doSomething();
}

enum MyEnum {
    A, B
}

record MyRecord(int x, int y) {}

@interface MyAnnotation {
    String value();
}
