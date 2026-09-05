package fixtures.service;

import java.util.Objects;

public class Service<T extends Number> {
    private final T current;

    public Service(T current) {
        this.current = Objects.requireNonNull(current);
    }

    public T current() {
        return current;
    }

    public static class Builder<U extends Number> {
        private U value;

        public Builder<U> value(U value) {
            this.value = value;
            return this;
        }

        public Service<U> build() {
            return new Service<>(value);
        }
    }
}
