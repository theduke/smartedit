package main

type MyStruct struct {
	field int
}

func NewMyStruct() *MyStruct {
	return &MyStruct{field: 0}
}

func (m *MyStruct) MyMethod() {
}

type MyInterface interface {
	AbstractMethod()
}
