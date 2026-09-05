package fixtures.kotlin

import java.time.Instant

enum class Status {
    NEW,
    READY,
}

data class Greeter(val prefix: String = "Hello") {
    fun greet(name: String): String {
        return "$prefix, $name!"
    }

    object Defaults {
        fun create(): Greeter = Greeter()
    }
}

class Registry {
    private val entries = mutableListOf<String>()

    fun size(): Int = entries.size
}

fun topLevel(value: Int): String = "value=$value at ${Instant.EPOCH}"
