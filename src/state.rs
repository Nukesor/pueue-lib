use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_derive::{Deserialize, Serialize};

use crate::error::Error;
use crate::network::message::{create_failure_message, Message};
use crate::settings::{Settings, PUEUE_DEFAULT_GROUP};
use crate::task::{Task, TaskStatus};

pub type SharedState = Arc<Mutex<State>>;

/// Represents the current status of a group.
/// Each group acts as a queue and can be managed individually.
#[derive(PartialEq, Clone, Debug, Deserialize, Serialize)]
pub enum GroupStatus {
    Running,
    Paused,
}

impl Default for GroupStatus {
    fn default() -> Self {
        Self::Running
    }
}

#[derive(PartialEq, Clone, Debug, Deserialize, Serialize)]
pub struct GroupInfo {
    pub status: GroupStatus,
    pub parallel_tasks: usize,
}

impl Default for GroupInfo {
    fn default() -> Self {
        Self {
            status: GroupStatus::default(),
            parallel_tasks: 1,
        }
    }
}

/// This is the full representation of the current state of the Pueue daemon.
///
/// This includes
/// - The currently used settings.
/// - The full task list
/// - The current status of all tasks
/// - All known groups.
///
/// However, the State does NOT include:
/// - Information about child processes
/// - Handles to child processes
///
/// That information is saved in the daemon's TaskHandler.
///
/// Most functions implemented on the state shouldn't be used by third party software.
/// The daemon is constantly changing and persisting the state. \
/// Any changes applied to a state and saved to disk, will most likely be overwritten
/// after a short time.
///
///
/// The daemon uses the state as a piece of shared memory between it's threads.
/// It's wrapped in a MutexGuard, which allows us to guarantee sequential access to any crucial
/// information, such as status changes and incoming commands by the client.
#[derive(PartialEq, Clone, Debug, Deserialize, Serialize)]
pub struct State {
    /// The current settings used by the daemon.
    pub settings: Settings,
    /// All tasks currently managed by the daemon.
    pub tasks: BTreeMap<usize, Task>,
    /// All groups
    pub groups: BTreeMap<String, GroupInfo>,
    /// Used to store an configuration path that has been explicitely specified.
    /// Without this, the default config path will be used instead.
    pub config_path: Option<PathBuf>,
}

impl State {
    /// Create a new default state.
    pub fn new(settings: &Settings, config_path: Option<PathBuf>) -> State {
        // Create a default group state.
        let mut groups = BTreeMap::new();
        if let Some(ref settings_groups) = settings.daemon.groups {
            for (name, parallel_tasks) in settings_groups.iter() {
                groups.insert(
                    name.clone(),
                    GroupInfo {
                        parallel_tasks: *parallel_tasks,
                        ..Default::default()
                    },
                );
            }
        }
        if !groups.contains_key(PUEUE_DEFAULT_GROUP) {
            groups.insert(PUEUE_DEFAULT_GROUP.to_string(), GroupInfo::default());
        }
        let mut state = State {
            settings: settings.clone(),
            tasks: BTreeMap::new(),
            groups,
            config_path,
        };
        state.create_group(PUEUE_DEFAULT_GROUP, 1);
        state
    }

    /// Add a new task
    pub fn add_task(&mut self, mut task: Task) -> usize {
        let next_id = match self.tasks.keys().max() {
            None => 0,
            Some(id) => id + 1,
        };
        task.id = next_id;
        self.tasks.insert(next_id, task);

        next_id
    }

    /// A small helper to change the status of a specific task.
    pub fn change_status(&mut self, id: usize, new_status: TaskStatus) {
        if let Some(ref mut task) = self.tasks.get_mut(&id) {
            task.status = new_status;
        };
    }

    /// Add a new group to the daemon. \
    /// This also check if the given group already exists.
    pub fn create_group(&mut self, group: &str, parallel_tasks: usize) {
        if self.groups.get(group).is_none() {
            self.groups.insert(
                group.into(),
                GroupInfo {
                    status: GroupStatus::Running,
                    parallel_tasks,
                },
            );
        }
    }

    /// Remove a group.
    /// This also iterates through all tasks and sets any tasks' group
    /// to the `default` group if it matches the deleted group.
    pub fn remove_group(&mut self, group: &str) -> Result<(), Error> {
        if group.eq(PUEUE_DEFAULT_GROUP) {
            return Err(Error::Generic(
                "You cannot remove the default group.".into(),
            ));
        }

        self.groups.remove(group);

        // Reset all tasks with removed group to the default.
        for (_, task) in self.tasks.iter_mut() {
            if task.group.eq(group) {
                task.set_default_group();
            }
        }

        Ok(())
    }

    /// Get an immutable reference to the specified group in this state, if it exists, otherwise
    /// return a failure message.
    pub fn get_group<'a, 'b>(&'a self, group: &'b str) -> Result<&'a GroupInfo, Message> {
        self.groups.get(group).ok_or_else(|| {
            create_failure_message(format!(
                "Group {} doesn't exists. Use one of these: {:?}",
                group,
                self.groups.keys()
            ))
        })
    }

    /// Get a mutable reference to the specified group in this state, if it exists, otherwise
    /// return a failure message.
    pub fn get_group_mut<'a, 'b>(
        &'a mut self,
        group: &'b str,
    ) -> Result<&'a mut GroupInfo, Message> {
        let failure_msg = create_failure_message(format!(
            "Group {} doesn't exists. Use one of these: {:?}",
            group,
            self.groups.keys()
        ));
        self.groups.get_mut(group).ok_or(failure_msg)
    }

    /// Set the group status (running/paused) for all groups including the default queue.
    pub fn set_status_for_all_groups(&mut self, status: GroupStatus) {
        for group in self.groups.values_mut() {
            group.status = status.clone();
        }
    }

    /// Get all ids of task inside a specific group.
    pub fn task_ids_in_group(&self, group: &str) -> Vec<usize> {
        self.tasks
            .iter()
            .filter(|(_, task)| task.group.eq(group))
            .map(|(id, _)| *id)
            .collect()
    }

    /// This checks, whether some tasks match the expected filter criteria. \
    /// The first result is the list of task_ids that match these statuses. \
    /// The second result is the list of task_ids that don't match these statuses. \
    ///
    /// By default, this checks all tasks in the current state. If a list of task_ids is
    /// provided as the third parameter, only those tasks will be checked.
    pub fn filter_tasks<F>(
        &self,
        filter: F,
        task_ids: Option<Vec<usize>>,
    ) -> (Vec<usize>, Vec<usize>)
    where
        F: Fn(&Task) -> bool,
    {
        // Either use all tasks or only the exlicitely specified ones.
        let task_ids = match task_ids {
            Some(ids) => ids,
            None => self.tasks.keys().cloned().collect(),
        };

        self.filter_task_ids(task_ids, filter)
    }

    /// Same as [tasks_in_statuses], but only checks for tasks of a specific group.
    pub fn filter_tasks_of_group<F>(&self, filter: F, group: &str) -> (Vec<usize>, Vec<usize>)
    where
        F: Fn(&Task) -> bool,
    {
        // Return empty vectors, if there's no such group.
        if !self.groups.contains_key(group) {
            return (vec![], vec![]);
        }

        // Filter all task ids of tasks that match the given group.
        let task_ids = self
            .tasks
            .iter()
            .filter(|(_, task)| task.group == group)
            .map(|(id, _)| *id)
            .collect();

        self.filter_task_ids(task_ids, filter)
    }

    /// Internal function used to check which of the given tasks match the provided filter.
    ///
    /// Returns a tuple of all (matching_task_ids, non_matching_task_ids).
    fn filter_task_ids<F>(&self, task_ids: Vec<usize>, filter: F) -> (Vec<usize>, Vec<usize>)
    where
        F: Fn(&Task) -> bool,
    {
        let mut matching = Vec::new();
        let mut mismatching = Vec::new();

        // Filter all task id's that match the provided statuses.
        for task_id in task_ids.iter() {
            // Check whether the task exists and save all non-existing task ids.
            match self.tasks.get(task_id) {
                None => {
                    mismatching.push(*task_id);
                    continue;
                }
                Some(task) => {
                    // Check whether the task status matches the filter.
                    if filter(task) {
                        matching.push(*task_id);
                    } else {
                        mismatching.push(*task_id);
                    }
                }
            };
        }

        (matching, mismatching)
    }
}
