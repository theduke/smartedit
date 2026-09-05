pub struct MyStruct {
    field: i32,
}

impl MyStruct {
    pub fn new() -> Self {
        Self { field: 0 }
    }

    pub fn my_method(&self) {}
}

pub trait MyTrait {
    fn abstract_method();
}

pub enum MyEnum {
    A,
}
