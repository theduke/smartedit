#include <string>

namespace math {

template <typename T>
class Box {
public:
    explicit Box(T value);
    T get() const;
    void set(T value);

private:
    T value_;
};

class Shape {
public:
    virtual double area() const = 0;
    void (*callback)(int);
};

enum class Color { Red, Green, Blue };

using StringBox = Box<std::string>;

template <typename T>
T identity(T value) {
    return value;
}

int add(int left, int right) {
    return left + right;
}

} // namespace math
