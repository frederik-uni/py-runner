# py-runner

Simple tool that allows you to execute Python code from Rust.

```rs
let x = execute_code("x = '10'", |_, globals| {
    globals.get_item("x")?.unwrap().extract::<String>()
}).unwrap();

let project1 = PythonModule::new_project(Path::new("./my-project/main.py").into()).unwrap();
let sum = project1.action(|_, module| module.call_method1("add", (1, 2))?.extract::<i64>()).unwrap();


let module1 = PythonModule::new_module(Path::new("./my-module")).unwrap();
let sum = module1.action(|_, module| module.call_method1("add", (1, 2))?.extract::<i64>()).unwrap();
```
