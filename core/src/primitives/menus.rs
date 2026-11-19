//! The menus as Lisp reaches them: adding entries to the menu bar and to the
//! context menus, taking them away, reading a menu back as it would open
//! now, and the commands that open one from the keyboard.
//!
//! Design: `doc/architettura-editor.typ, <menu>`.
use super::*;
use crate::editor::menus::menu_command;
use crate::managers::{MenuEntry, MenuOwner, MenuPathError, SEPARATOR_PREFIX};
use crate::ui::{Menu, MenuItem, MenuKind};

/// Returns ARG as a path of labels: a list of strings, or one string for a
/// path of one. Every label must be a string with something in it.
fn path_arg<B: BufferTrait>(
    arg: Option<&ELispExp<B>>,
) -> Result<Vec<String>, EvalError<EditorState<B>>> {
    let bad = |got: &ELispExp<B>| EvalError::WrongArgumentType {
        expected: "String or list of strings".into(),
        got: got.clone(),
    };
    let items = match arg {
        None => return Ok(Vec::new()),
        Some(ELispExp::String(label)) => vec![ELispExp::String(label.clone())],
        Some(other) if other.is_nil() => Vec::new(),
        Some(other @ (ELispExp::Cons(_) | ELispExp::Form(_))) => args::list(other),
        Some(other) => return Err(bad(other)),
    };
    items
        .iter()
        .map(|item| match item {
            ELispExp::String(label) if !label.is_empty() => Ok(label.to_string()),
            other => Err(bad(other)),
        })
        .collect()
}

/// Returns ARG as the mode a context menu belongs to: nil for every buffer.
fn mode_arg<B: BufferTrait>(
    arg: Option<&ELispExp<B>>,
) -> Result<MenuOwner, EvalError<EditorState<B>>> {
    Ok(MenuOwner::Context(args::optional_name(
        arg,
        "Symbol naming a mode, or nil",
    )?))
}

/// Returns the error NAME reports for a path that could not take an entry.
fn path_error<B: BufferTrait>(name: &str, error: MenuPathError) -> EvalError<EditorState<B>> {
    EvalError::RuntimeMessage(match error {
        MenuPathError::Empty => format!("{name}: the path names no entry"),
        MenuPathError::NotAMenu(label) => format!("{name}: {label} is an entry, not a menu"),
        MenuPathError::KindClash(label) => {
            format!("{name}: {label} is already there, as a menu or as an entry")
        }
    })
}

/// Adds, for NAME, the entry at the end of PATH to OWNER's menu: a separator
/// when its label begins with `--`, otherwise COMMAND, greyed out while
/// ENABLE says nil. Returns t.
fn add_entry<B: BufferTrait>(
    name: &str,
    owner: MenuOwner,
    path: Vec<String>,
    command: Option<&ELispExp<B>>,
    enable: Option<&ELispExp<B>>,
    ctx: &EditorState<B>,
) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
    let Some((label, menus)) = path.split_last() else {
        return Err(path_error(name, MenuPathError::Empty));
    };
    if let Some(menu) = menus.iter().find(|menu| menu.starts_with(SEPARATOR_PREFIX)) {
        return Err(path_error(name, MenuPathError::NotAMenu(menu.clone())));
    }
    let entry = if label.starts_with(SEPARATOR_PREFIX) {
        MenuEntry::Separator {
            label: label.clone(),
        }
    } else {
        let given = command.cloned().unwrap_or_else(ELispExp::nil);
        let Some(command) = menu_command(&given) else {
            return Err(EvalError::WrongArgumentType {
                expected: "Command name, function or form".into(),
                got: given,
            });
        };
        MenuEntry::Item {
            label: label.clone(),
            command,
            enable: enable.filter(|enable| !enable.is_nil()).cloned(),
        }
    };
    ctx.menus_mut(|registry| registry.add(&owner, menus, entry))
        .map_err(|error| path_error(name, error))?;
    Ok(ELispExp::t())
}

/// Adds, for NAME, FUNCTION as the generator of entries of the menu at PATH
/// in OWNER's menu. Returns t.
fn add_generator<B: BufferTrait>(
    name: &str,
    owner: MenuOwner,
    path: Vec<String>,
    function: Option<&ELispExp<B>>,
    ctx: &EditorState<B>,
) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
    let function = match function {
        Some(function @ (ELispExp::Symbol(_) | ELispExp::Lambda(_))) if !function.is_nil() => {
            function.clone()
        }
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "Function".into(),
                got: other.cloned().unwrap_or_else(ELispExp::nil),
            });
        }
    };
    let key = match &function {
        ELispExp::Symbol(symbol) => symbol.to_string(),
        other => format!("{other:?}"),
    };
    let entry = MenuEntry::Generated {
        name: key,
        function,
    };
    ctx.menus_mut(|menus| menus.add(&owner, &path, entry))
        .map_err(|error| path_error(name, error))?;
    Ok(ELispExp::t())
}

/// Returns MENU as `menu-bar-items` gives it: a separator as "--", an
/// action as (LABEL COMMAND ENABLED KEYS), a menu as (LABEL ITEMS).
fn menu_as_lisp<B: BufferTrait>(menu: &Menu, ctx: &EditorState<B>) -> ELispExp<B> {
    ELispExp::proper_list(
        menu.items
            .iter()
            .map(|item| match item {
                MenuItem::Separator => ELispExp::string(SEPARATOR_PREFIX.to_string()),
                MenuItem::Action {
                    label,
                    enabled,
                    keys,
                    id,
                } => ELispExp::proper_list(vec![
                    ELispExp::string(label.clone()),
                    ctx.menu_item_command(*id),
                    ELispExp::boolean(*enabled),
                    if keys.is_empty() {
                        ELispExp::nil()
                    } else {
                        ELispExp::string(keys.clone())
                    },
                ]),
                MenuItem::Submenu { label, menu } => ELispExp::proper_list(vec![
                    ELispExp::string(label.clone()),
                    menu_as_lisp(menu, ctx),
                ]),
            })
            .collect(),
    )
}

pub const MENU_BAR_ADD_DOC: &str = "(menu-bar-add PATH COMMAND &optional ENABLE): Add to the menu \
         bar the entry PATH ends with, a list of labels such as '(\"File\" \"Open...\"), and \
         return t. The menus on the way are made; an entry already there with the label is \
         replaced. COMMAND is a command's name or a form; the entry is greyed out while ENABLE, \
         a function's name or a form, says nil. A label that begins with -- is a separator.\n\n\
         Example:\n\
         (menu-bar-add '(\"File\" \"Revert\") 'revert-buffer 'buffer-file-name)";

primitive!(menu_bar_add, args, _env, ctx, {
    if !(2..=3).contains(&args.len()) {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let path = path_arg(args.first())?;
    add_entry(
        "menu-bar-add",
        MenuOwner::Bar,
        path,
        args.get(1),
        args.get(2),
        ctx,
    )
});

pub const CONTEXT_MENU_ADD_DOC: &str = "(context-menu-add MODE PATH COMMAND &optional ENABLE): Add \
         to the context menu of the buffers where MODE is on, or of every buffer when MODE is \
         nil, the entry PATH ends with, as `menu-bar-add' adds one to the menu bar; return t.\n\n\
         Example:\n\
         (context-menu-add 'occur-mode \"Visit\" 'occur-goto)";

primitive!(context_menu_add, args, _env, ctx, {
    if !(3..=4).contains(&args.len()) {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 3,
            got: args.len(),
        });
    }
    let owner = mode_arg(args.first())?;
    let path = path_arg(args.get(1))?;
    add_entry(
        "context-menu-add",
        owner,
        path,
        args.get(2),
        args.get(3),
        ctx,
    )
});

pub const MENU_BAR_ADD_DYNAMIC_DOC: &str = "(menu-bar-add-dynamic PATH FUNCTION): Fill the menu at \
         PATH in the menu bar, each time it opens, with the entries FUNCTION returns, after those \
         before it; return t. Each is (LABEL COMMAND), (LABEL COMMAND ENABLED) or a string \
         beginning with -- for a separator.\n\n\
         Example:\n\
         (defun my-entries () (list (list \"Scratch\" '(switch-to-buffer \"*scratch*\"))))\n\
         (menu-bar-add-dynamic \"Buffers\" 'my-entries)";

primitive!(menu_bar_add_dynamic, args, _env, ctx, {
    exact_arity(args, 2)?;
    let path = path_arg(args.first())?;
    add_generator(
        "menu-bar-add-dynamic",
        MenuOwner::Bar,
        path,
        args.get(1),
        ctx,
    )
});

pub const CONTEXT_MENU_ADD_DYNAMIC_DOC: &str = "(context-menu-add-dynamic MODE PATH FUNCTION): \
         Fill the menu at PATH in MODE's context menu, the context menu itself when PATH is nil, \
         with the entries FUNCTION returns each time it opens, as `menu-bar-add-dynamic' does; \
         return t.";

primitive!(context_menu_add_dynamic, args, _env, ctx, {
    exact_arity(args, 3)?;
    let owner = mode_arg(args.first())?;
    let path = path_arg(args.get(1))?;
    add_generator("context-menu-add-dynamic", owner, path, args.get(2), ctx)
});

pub const MENU_BAR_REMOVE_DOC: &str = "(menu-bar-remove PATH): Take away the entry at PATH in the \
         menu bar, a menu with everything in it, and return t, or nil when there was none.\n\n\
         Example:\n\
         (menu-bar-remove \"Help\")";

primitive!(menu_bar_remove, args, _env, ctx, {
    exact_arity(args, 1)?;
    let path = path_arg(args.first())?;
    Ok(ELispExp::boolean(
        ctx.menus_mut(|menus| menus.remove(&MenuOwner::Bar, &path)),
    ))
});

pub const CONTEXT_MENU_REMOVE_DOC: &str = "(context-menu-remove MODE PATH): Take away the entry \
         at PATH in MODE's context menu, or in that of every buffer when MODE is nil, and return \
         t, or nil when there was none.";

primitive!(context_menu_remove, args, _env, ctx, {
    exact_arity(args, 2)?;
    let owner = mode_arg(args.first())?;
    let path = path_arg(args.get(1))?;
    Ok(ELispExp::boolean(
        ctx.menus_mut(|menus| menus.remove(&owner, &path)),
    ))
});

pub const MENU_BAR_ITEMS_DOC: &str = "(menu-bar-items): Return the menu bar as it would open now \
         in the current buffer: a separator as \"--\", an entry as (LABEL COMMAND ENABLED KEYS), \
         KEYS nil when no key runs COMMAND, and a menu as (LABEL ITEMS).\n\n\
         Example:\n\
         (mapcar 'car (menu-bar-items))   ; (\"File\" \"Buffers\" \"Help\")";

primitive!(menu_bar_items, args, env, ctx, {
    exact_arity(args, 0)?;
    Ok(menu_as_lisp(&ctx.menu_bar(&env), ctx))
});

pub const CONTEXT_MENU_ITEMS_DOC: &str = "(context-menu-items): Return the context menu as it \
         would open now in the current buffer, in the shape `menu-bar-items' gives.";

primitive!(context_menu_items, args, env, ctx, {
    exact_arity(args, 0)?;
    Ok(menu_as_lisp(&ctx.context_menu(&env), ctx))
});

/// Asks the front end to open the menu KIND names, and returns t if it was
/// asked.
fn open_menu<B: BufferTrait>(
    kind: MenuKind,
    env: &std::sync::Arc<Env<EditorState<B>>>,
    ctx: &EditorState<B>,
) -> ELispExp<B> {
    ELispExp::boolean(ctx.request_menu(kind, None, env))
}

pub const MENU_BAR_OPEN_DOC: &str = "(menu-bar-open): Open the first menu of the menu bar, for \
         the keyboard to walk, whether the bar is shown or not; return t, or nil when there is \
         nothing to open.";

primitive!(menu_bar_open, _args, env, ctx, {
    Ok(open_menu(MenuKind::Bar { open: 0 }, &env, ctx))
});

pub const CONTEXT_MENU_OPEN_DOC: &str = "(context-menu-open): Open the context menu of the \
         current buffer at point, as a right click opens it under the pointer; return t, or nil \
         when it is empty.";

primitive!(context_menu_open, _args, env, ctx, {
    Ok(open_menu(MenuKind::Context, &env, ctx))
});

/// Registers the primitives of the menus.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("menu-bar-add", menu_bar_add, MENU_BAR_ADD_DOC);
    into.function(
        "menu-bar-add-dynamic",
        menu_bar_add_dynamic,
        MENU_BAR_ADD_DYNAMIC_DOC,
    );
    into.function("menu-bar-remove", menu_bar_remove, MENU_BAR_REMOVE_DOC);
    into.function("menu-bar-items", menu_bar_items, MENU_BAR_ITEMS_DOC);
    into.function("context-menu-add", context_menu_add, CONTEXT_MENU_ADD_DOC);
    into.function(
        "context-menu-add-dynamic",
        context_menu_add_dynamic,
        CONTEXT_MENU_ADD_DYNAMIC_DOC,
    );
    into.function(
        "context-menu-remove",
        context_menu_remove,
        CONTEXT_MENU_REMOVE_DOC,
    );
    into.function(
        "context-menu-items",
        context_menu_items,
        CONTEXT_MENU_ITEMS_DOC,
    );
    // Commands, so that a key opens a menu as it runs anything else.
    into.command("menu-bar-open", menu_bar_open, &[], MENU_BAR_OPEN_DOC);
    into.command(
        "context-menu-open",
        context_menu_open,
        &[],
        CONTEXT_MENU_OPEN_DOC,
    );
}
