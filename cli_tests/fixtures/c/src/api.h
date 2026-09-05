typedef unsigned long Size;

typedef struct Point {
    int x;
    int y;
} Point;

typedef union Value {
    int integer;
    float decimal;
} Value;

enum Color {
    COLOR_RED,
    COLOR_BLUE,
};

int manhattan(Point point);
int (*callback)(int);
