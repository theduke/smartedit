package fixtures.service;

public interface Contracts<T> {
    T transform(T value);

    default String label(T value) {
        return "contract:" + value;
    }

    enum Status {
        ACTIVE,
        DISABLED
    }

    record Result<T>(T value, Status status) {
        public boolean active() {
            return status == Status.ACTIVE;
        }
    }

    @interface Marker {
        String name() default "service";
        int priority() default 1;
    }
}
