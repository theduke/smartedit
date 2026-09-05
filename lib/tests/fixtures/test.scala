package com.example

class MyClass(val field: Int) {
    def myMethod(arg: String): Unit = {
        println(arg)
    }
}

object MyObject {
    def doSomething(): Unit = {}
}

trait MyTrait {
    def abstractMethod(): Int
}
