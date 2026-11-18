//! Which minor modes each buffer role admits: the rules, and the one answer
//! read from them. Kept in [`super::Modes`], beside the minor modes they are
//! about; asked by `EditorState::minor_mode_on` and by nothing else, so that a
//! command, a rule of `minor-mode-enable` and a global mode get the same
//! answer.
//!
//! Design: `doc/architettura-editor.typ, <ruoli>`.
use crate::buffer::{BufferRole, READ_ONLY_MODE};
use std::collections::HashMap;

/// Who wrote a rule. Of two rules for the same role and minor mode, the
/// user's wins over the one the editor ships with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PolicyOrigin {
    /// The modules the editor ships with.
    Distributed,
    /// The user's own configuration.
    User,
}

impl PolicyOrigin {
    /// Returns the words a refusal uses for it.
    pub fn described(self) -> &'static str {
        match self {
            PolicyOrigin::Distributed => "the distributed configuration",
            PolicyOrigin::User => "the user configuration",
        }
    }
}

/// What a rule says of a minor mode in a role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    /// Admitted.
    Permit,
    /// Kept out.
    Revoke,
}

/// What settled whether a role admits a minor mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecidedBy {
    /// The core's own minor mode, `read-only-mode`, which every role admits:
    /// it protects a buffer, and is no feature of one.
    Core,
    /// The rule of ORIGIN for ROLE, the nearest role with one.
    Rule {
        role: BufferRole,
        origin: PolicyOrigin,
    },
    /// No rule: what the root role ROLE does with what no rule speaks of.
    Default { role: BufferRole },
}

/// Whether a role admits a minor mode, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    pub admitted: bool,
    pub by: DecidedBy,
}

/// A minor mode a buffer did not admit: what `minor-mode-on` refused, with
/// the reason to give.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub minor: String,
    pub buffer: String,
    pub role: BufferRole,
    pub by: DecidedBy,
}

impl Refusal {
    /// Returns the sentence a command shows for it.
    pub fn reason(&self) -> String {
        let Refusal {
            minor,
            buffer,
            role,
            by,
        } = self;
        let name = role.name();
        match by {
            DecidedBy::Rule { role: at, origin } if at == role => format!(
                "{minor} is not admitted in {buffer}: {} revokes it for the {name} role",
                origin.described()
            ),
            DecidedBy::Rule { role: at, origin } => format!(
                "{minor} is not admitted in {buffer}: {} revokes it for the {} role, \
                 which the {name} role derives from",
                origin.described(),
                at.name()
            ),
            DecidedBy::Default { .. } | DecidedBy::Core => format!(
                "{minor} is not admitted in {buffer}: the {name} role admits only the \
                 minor modes permitted for it"
            ),
        }
    }
}

/// The two rules a role can have for one minor mode, one from each origin.
#[derive(Clone, Copy, Debug, Default)]
struct Rules {
    distributed: Option<Admission>,
    user: Option<Admission>,
}

impl Rules {
    fn slot(&mut self, origin: PolicyOrigin) -> &mut Option<Admission> {
        match origin {
            PolicyOrigin::Distributed => &mut self.distributed,
            PolicyOrigin::User => &mut self.user,
        }
    }

    /// Returns the rule in force, the user's first, and whose it is.
    fn winner(&self) -> Option<(Admission, PolicyOrigin)> {
        self.user.map(|rule| (rule, PolicyOrigin::User)).or(self
            .distributed
            .map(|rule| (rule, PolicyOrigin::Distributed)))
    }
}

/// The rules of every role, by role and minor mode.
#[derive(Debug, Default)]
pub struct MinorPolicies {
    rules: HashMap<(BufferRole, String), Rules>,
}

impl MinorPolicies {
    /// Records that ORIGIN says ADMISSION of MINOR in ROLE, in place of what
    /// ORIGIN said before. Returns whether that changed anything.
    pub fn set(
        &mut self,
        role: BufferRole,
        minor: &str,
        origin: PolicyOrigin,
        admission: Admission,
    ) -> bool {
        let slot = self
            .rules
            .entry((role, minor.to_string()))
            .or_default()
            .slot(origin);
        slot.replace(admission) != Some(admission)
    }

    /// Forgets what ORIGIN said of MINOR in ROLE. Returns whether it had
    /// said anything.
    pub fn clear(&mut self, role: BufferRole, minor: &str, origin: PolicyOrigin) -> bool {
        let key = (role, minor.to_string());
        let Some(rules) = self.rules.get_mut(&key) else {
            return false;
        };
        let had = rules.slot(origin).take().is_some();
        if rules.distributed.is_none() && rules.user.is_none() {
            self.rules.remove(&key);
        }
        had
    }

    /// Returns what ORIGIN says of MINOR in ROLE itself, with nothing
    /// inherited.
    pub fn rule(&self, role: BufferRole, minor: &str, origin: PolicyOrigin) -> Option<Admission> {
        let mut rules = *self.rules.get(&(role, minor.to_string()))?;
        *rules.slot(origin)
    }

    /// Returns whether a buffer of ROLE admits MINOR: the rule of the nearest
    /// role, ROLE itself first and then those it derives from, the user's
    /// where both origins have one; with no rule, what the root role does.
    pub fn decide(&self, role: BufferRole, minor: &str) -> Decision {
        if minor == READ_ONLY_MODE {
            return Decision {
                admitted: true,
                by: DecidedBy::Core,
            };
        }
        for at in role.lineage() {
            if let Some((admission, origin)) = self
                .rules
                .get(&(at, minor.to_string()))
                .and_then(Rules::winner)
            {
                return Decision {
                    admitted: admission == Admission::Permit,
                    by: DecidedBy::Rule { role: at, origin },
                };
            }
        }
        Decision {
            admitted: role.open_by_default(),
            by: DecidedBy::Default { role: role.root() },
        }
    }
}

/// The roles an editor knows by name: the five it ships and those its
/// configuration defined with `define-buffer-role`. Kept in
/// [`super::Modes`] beside the rules that speak of them.
#[derive(Debug, Default)]
pub struct RoleRegistry {
    defined: HashMap<String, BufferRole>,
}

/// Why `define-buffer-role` would not define a role.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoleDefinitionError {
    /// NAME is a role already, deriving from something else: a role is
    /// fixed for the life of the buffers that have it.
    Incompatible {
        name: String,
        parent: Option<BufferRole>,
        wanted: Option<BufferRole>,
    },
}

impl RoleDefinitionError {
    /// Returns the sentence an error shows for it.
    pub fn reason(&self) -> String {
        let derived = |parent: &Option<BufferRole>| match parent {
            Some(parent) => format!("deriving from {}", parent.name()),
            None => "deriving from no role".to_string(),
        };
        match self {
            RoleDefinitionError::Incompatible {
                name,
                parent,
                wanted,
            } => format!(
                "the buffer role {name} exists already, {}, not {}",
                derived(parent),
                derived(wanted)
            ),
        }
    }
}

impl RoleRegistry {
    /// Returns the role called NAME: one the editor ships, or one defined.
    pub fn named(&self, name: &str) -> Option<BufferRole> {
        BufferRole::from_name(name).or_else(|| self.defined.get(name).copied())
    }

    /// Defines the role NAME, deriving from PARENT, or a root of its own
    /// when PARENT is `None`. Returns the role, and whether it is new: the
    /// same definition made again changes nothing; one that would give NAME
    /// another parent, a role the editor ships included, is refused.
    pub fn define(
        &mut self,
        name: &str,
        parent: Option<BufferRole>,
    ) -> Result<(BufferRole, bool), RoleDefinitionError> {
        if let Some(existing) = self.named(name) {
            if existing.parent() == parent {
                return Ok((existing, false));
            }
            return Err(RoleDefinitionError::Incompatible {
                name: name.to_string(),
                parent: existing.parent(),
                wanted: parent,
            });
        }
        let role = BufferRole::Defined(crate::buffer::role::DefinedRole::interned(name, parent));
        self.defined.insert(name.to_string(), role);
        Ok((role, true))
    }

    /// Returns every role known, the ones the editor ships first, then the
    /// defined ones by name.
    pub fn all(&self) -> Vec<BufferRole> {
        let mut defined: Vec<BufferRole> = self.defined.values().copied().collect();
        defined.sort_by_key(|role| role.name());
        BufferRole::ALL.into_iter().chain(defined).collect()
    }
}
