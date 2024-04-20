use std::collections::{btree_map, BTreeMap};
use std::{mem, thread};

use serde::Serialize;

use crate::thread_rt::{Builder, ScopedTask, Task};
use crate::time::Interval;
use crate::{Error, Result};

pub mod prelude {
    pub use super::Supervisor;
    pub use crate::thread_rt::{Builder, Scheduling};
}

/// A supervisor object used to manage tasks spawned with [`Builder`]
#[derive(Serialize)]
pub struct Supervisor<T> {
    tasks: BTreeMap<String, Task<T>>,
}

impl<T> Default for Supervisor<T> {
    fn default() -> Self {
        Self {
            tasks: <_>::default(),
        }
    }
}

macro_rules! vacant_entry {
    ($self:ident, $builder:ident) => {{
        let Some(name) = $builder.name.clone() else {
                        return Err(Error::SupervisorNameNotSpecified);
                    };
        let btree_map::Entry::Vacant(entry) = $self.tasks.entry(name.clone()) else {
                        return Err(Error::SupervisorDuplicateTask(name));
                    };
        entry
    }};
}

impl<T> Supervisor<T> {
    pub fn new() -> Self {
        Self::default()
    }
    /// Spawns a new task using a [`Builder`] object and registers it. The task name MUST be unique
    /// and SHOULD be 15 characters or less to set a proper thread name
    pub fn spawn<F, B>(&mut self, builder: B, f: F) -> Result<&Task<T>>
    where
        B: Into<Builder>,
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let builder = builder.into();
        let entry = vacant_entry!(self, builder);
        let task = builder.spawn(f)?;
        Ok(entry.insert(task))
    }
    /// Spawns a new periodic task using a [`Builder`] object and registers it. The task name MUST
    /// be unique and SHOULD be 15 characters or less to set a proper thread name
    pub fn spawn_periodic<F, B>(&mut self, builder: B, f: F, interval: Interval) -> Result<&Task<T>>
    where
        F: Fn() -> T + Send + 'static,
        T: Send + 'static,
        B: Into<Builder>,
    {
        let builder = builder.into();
        let entry = vacant_entry!(self, builder);
        let task = builder.spawn_periodic(f, interval)?;
        Ok(entry.insert(task))
    }
    /// Gets a task by its name
    pub fn get_task(&self, name: &str) -> Option<&Task<T>> {
        self.tasks.get(name)
    }
    /// Gets a task by its name as a mutable object
    pub fn get_task_mut(&mut self, name: &str) -> Option<&mut Task<T>> {
        self.tasks.get_mut(name)
    }
    /// Takes a task by its name and removes it from the internal registry
    pub fn take_task(&mut self, name: &str) -> Option<Task<T>> {
        self.tasks.remove(name)
    }
    /// Removes a task from the internal registry
    pub fn forget_task(&mut self, name: &str) -> Result<()> {
        if self.tasks.remove(name).is_some() {
            Ok(())
        } else {
            Err(Error::SupervisorTaskNotFound)
        }
    }
    /// Removes all finished tasks from the internal registry
    pub fn purge(&mut self) {
        self.tasks.retain(|_, task| !task.is_finished());
    }
    /// Joins all tasks in the internal registry and returns a map with their results. After the
    /// operation the registry is cleared
    pub fn join_all(&mut self) -> BTreeMap<String, thread::Result<T>> {
        let mut result = BTreeMap::new();
        for (name, task) in mem::take(&mut self.tasks) {
            if !task.is_blocking() {
                result.insert(name, task.join());
            }
        }
        result
    }
}

#[allow(clippy::module_name_repetitions)]
#[derive(Serialize)]
pub struct ScopedSupervisor<'a, 'env: 'a, T> {
    tasks: BTreeMap<String, ScopedTask<'a, T>>,
    #[serde(skip_serializing)]
    scope: &'a thread::Scope<'a, 'env>,
}

impl<'a, 'env, T> ScopedSupervisor<'a, 'env, T> {
    pub fn new(scope: &'a thread::Scope<'a, 'env>) -> Self {
        Self {
            tasks: <_>::default(),
            scope,
        }
    }
    /// Spawns a new task using a [`Builder`] object and registers it. The task name MUST be unique
    /// and SHOULD be 15 characters or less to set a proper thread name
    pub fn spawn<F, B>(&mut self, builder: B, f: F) -> Result<&ScopedTask<T>>
    where
        B: Into<Builder>,
        F: FnOnce() -> T + Send + 'a,
        T: Send + 'a,
    {
        let builder = builder.into();
        let entry = vacant_entry!(self, builder);
        let task = builder.spawn_scoped(self.scope, f)?;
        Ok(entry.insert(task))
    }
    /// Spawns a new periodic task using a [`Builder`] object and registers it. The task name MUST
    /// be unique and SHOULD be 15 characters or less to set a proper thread name
    pub fn spawn_periodic<F, B>(
        &mut self,
        builder: B,
        f: F,
        interval: Interval,
    ) -> Result<&ScopedTask<T>>
    where
        F: Fn() -> T + Send + 'a,
        T: Send + 'a,
        B: Into<Builder>,
    {
        let builder = builder.into();
        let entry = vacant_entry!(self, builder);
        let task = builder.spawn_scoped_periodic(self.scope, f, interval)?;
        Ok(entry.insert(task))
    }
    /// Gets a task by its name
    pub fn get_task(&self, name: &str) -> Option<&ScopedTask<T>> {
        self.tasks.get(name)
    }
    /// Gets a task by its name as a mutable object
    pub fn get_task_mut(&mut self, name: &str) -> Option<&mut ScopedTask<'a, T>> {
        self.tasks.get_mut(name)
    }
    /// Takes a task by its name and removes it from the internal registry
    pub fn take_task(&mut self, name: &str) -> Option<ScopedTask<T>> {
        self.tasks.remove(name)
    }
    /// Removes a task from the internal registry
    pub fn forget_task(&mut self, name: &str) -> Result<()> {
        if self.tasks.remove(name).is_some() {
            Ok(())
        } else {
            Err(Error::SupervisorTaskNotFound)
        }
    }
    /// Removes all finished tasks from the internal registry
    pub fn purge(&mut self) {
        self.tasks.retain(|_, task| !task.is_finished());
    }
    /// Joins all tasks in the internal registry and returns a map with their results. After the
    /// operation the registry is cleared
    pub fn join_all(&mut self) -> BTreeMap<String, thread::Result<T>> {
        let mut result = BTreeMap::new();
        for (name, task) in mem::take(&mut self.tasks) {
            if !task.is_blocking() {
                result.insert(name, task.join());
            }
        }
        result
    }
}
