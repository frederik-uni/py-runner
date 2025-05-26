use crossbeam::channel::{self, Sender};
use nanoid::nanoid;
use pyo3::Python;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::env;
use std::ffi::{CStr, CString};
use std::path::Path;
use std::path::PathBuf;
use std::thread;

/// sets env variable PYTHONPATH
/// `set_venv("./venv", "python3.11")`
pub fn set_venv(venv: &str, python_version: &str) {
    unsafe {
        env::set_var(
            "PYTHONPATH",
            format!("{venv}/lib/{python_version}/site-packages",),
        );
    }
}

pub struct PythonModule {
    task_sender: Sender<Option<Box<dyn FnOnce(&Python, &Bound<'_, PyAny>) + Send>>>,
    thread_handle: thread::JoinHandle<PyResult<()>>,
}

impl Drop for PythonModule {
    fn drop(&mut self) {
        self.task_sender.send(None).unwrap();
    }
}

impl PythonModule {
    /// Runs action on the imported module
    ///```rs
    /// module
    ///    .action(|py, module| module.call_method1("add", (1, 2))?.extract::<i64>())
    ///    .unwrap();
    /// ```
    pub fn action<T: Send + 'static>(
        &self,
        call: fn(&Python<'_>, &Bound<'_, PyAny>) -> PyResult<T>,
    ) -> PyResult<T> {
        if self.thread_handle.is_finished() {
            return Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "Python thread has exited",
            ));
        }

        let (sender, receiver) = std::sync::mpsc::sync_channel(1);

        let task: Box<dyn FnOnce(&Python, &Bound<'_, PyAny>) + Send> =
            Box::new(move |py: &Python, module: &Bound<'_, PyAny>| {
                let result = call(py, module);
                let _ = sender.send(result);
            });

        self.task_sender
            .send(Some(task))
            .map_err(|_| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Task send failed"))?;

        receiver.recv().unwrap()
    }

    /// Loads a Python module from a directory
    /// `let module = PythonModule::new_module(Path::new("./my-module")).unwrap();`
    pub fn new_module(path: &Path) -> PyResult<PythonModule> {
        let init_file = path.join("__init__.py");
        Self::new_project(init_file)
    }

    /// Loads a Python project from root file
    /// `let project = PythonModule::new_project(Path::new("./my-project/main.py").into()).unwrap()`
    pub fn new_project(init_file: PathBuf) -> PyResult<PythonModule> {
        if !init_file.is_file() {
            return Err(PyErr::new::<pyo3::exceptions::PyFileNotFoundError, _>(
                format!("No {} found", init_file.display()),
            ));
        }
        let module_name = nanoid!(16);
        let (task_sender, task_receiver) =
            channel::unbounded::<Option<Box<dyn FnOnce(&Python, &Bound<'_, PyAny>) + Send>>>();
        let (init_sender, init_receiver) = std::sync::mpsc::sync_channel::<PyResult<()>>(0);

        let thread_handle = thread::spawn(move || {
            let v: PyResult<()> = Python::with_gil(|py| {
                let init = || {
                    let importlib_util = PyModule::import(py, "importlib.util")?;

                    let spec = importlib_util
                        .getattr("spec_from_file_location")?
                        .call1((&module_name, init_file))?;

                    let module = importlib_util
                        .getattr("module_from_spec")?
                        .call1((spec.clone(),))?;
                    let sys = py.import("sys")?;
                    let modules = sys.getattr("modules")?;
                    modules.set_item(module_name, &module)?;
                    let loader = spec.getattr("loader")?;
                    loader.call_method1("exec_module", (module.clone(),))?;
                    Ok(module)
                };
                match init() {
                    Ok(module) => {
                        let _ = init_sender.send(Ok(()));
                        while let Ok(Some(task)) = py.allow_threads(|| task_receiver.recv()) {
                            task(&py, &module);
                        }
                    }
                    Err(e) => {
                        let _ = init_sender.send(Err(e));
                    }
                }

                Ok(())
            });
            v
        });
        if let Ok(v) = init_receiver.recv() {
            v?;
        }

        Ok(PythonModule {
            task_sender,
            thread_handle,
        })
    }
}

pub fn execute_code_(s: &str) -> PyResult<()> {
    execute_code::<()>(s, |_, _| Ok(()))
}

/// Runs Python code
pub fn execute_code<T>(
    s: &str,
    f: fn(Python<'_>, Bound<'_, PyDict>) -> PyResult<T>,
) -> PyResult<T> {
    Python::with_gil(|py| {
        let c_string = CString::new(s).expect("CString::new failed");

        let c_str: &CStr = c_string.as_c_str();
        let globals = PyDict::new(py);

        py.run(c_str, Some(&globals), None).unwrap();
        f(py, globals)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execute_code() {
        let x = execute_code("x = '10'", |_, globals| {
            globals.get_item("x")?.unwrap().extract::<String>()
        })
        .unwrap();

        assert_eq!(x, "10");
    }

    #[test]
    fn test_load_project() {
        let project1 = PythonModule::new_project(Path::new("./my-project/main.py").into()).unwrap();
        let sum = project1
            .action(|_, module| module.call_method1("add", (1, 2))?.extract::<i64>())
            .unwrap();
        assert_eq!(sum, 3)
    }

    #[test]
    fn test_load_module() {
        let module1 = PythonModule::new_module(Path::new("./my-module")).unwrap();
        let sum = module1
            .action(|_, module| module.call_method1("add", (1, 2))?.extract::<i64>())
            .unwrap();
        assert_eq!(sum, 3)
    }
}
