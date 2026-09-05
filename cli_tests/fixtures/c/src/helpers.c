#include "api.h"

int manhattan(Point point) {
    return point.x + point.y;
}

Size point_size(void) {
    return sizeof(Point);
}
