class Flag {
public:
    operator bool() const;
    Flag& operator=(const Flag& other);
};

using Predicate = bool (*)(int);

bool (*choose())(int) {
    return nullptr;
}
