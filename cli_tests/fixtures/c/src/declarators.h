int first(void), second(void);
int *allocate(void);
int (*handler)(int);
int (*handlers[2])(int);
int (*factory(void))(int);
