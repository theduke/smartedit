#include "api.h"

int add(int left, int right) {
    return left + right;
}

int main(void) {
    Point origin = {0, 0};
    return add(origin.x, origin.y);
}
