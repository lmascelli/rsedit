//! Loading a module once: `require` and `provide`, what is loaded and from
//! where, `eval-after-load`, and the plugins found at start.
//!
//! Design: `doc/architettura-editor.typ, <moduli-lisp>`.
use super::*;
use crate::managers::FeatureState;

pub const REQUIRE_DOC: &str = "(require FEATURE &optional FILE): Load FEATURE, a symbol, \
         unless it is loaded already, and return it; nil, with a line in the log, when there is \
         no file to load it from.\n\n\
         The file is FEATURE.lisp in the first directory of `lisp-path' that has one, or FILE \
         when given: a path, or a bare name looked for in the same way. It is evaluated as \
         `eval-file' evaluates, and must `provide' FEATURE. A file that requires, directly or \
         not, the feature it is being loaded for is an error; so is one that raises an error or \
         does not provide FEATURE, and the features it provided on the way are then not loaded \
         either.\n\n\
         Example:\n\
         (require 'dired)";

primitive!(require, args, env, ctx, {
    if args.is_empty() || args.len() > 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let feature = args::name(args.first())?;
    let file = args::optional_name(args.get(1), "String naming a file")?;
    ctx.require(&feature, file.as_deref(), &env)
});

pub const PROVIDE_DOC: &str = "(provide FEATURE): Record FEATURE, a symbol, as loaded, from the \
         file being evaluated, and return it. What waits for it with `eval-after-load' runs now. \
         A module ends with it.\n\n\
         Example:\n\
         (provide 'my-module)";

primitive!(provide, args, env, ctx, {
    exact_arity(args, 1)?;
    let feature = args::name(args.first())?;
    ctx.provide(&feature, &env);
    Ok(ELispExp::symbol(feature))
});

pub const FEATUREP_DOC: &str = "(featurep FEATURE): Return t if FEATURE has been provided and \
         is loaded, nil if not.\n\n\
         Example:\n\
         (when (featurep 'vim) (global-vim-mode t))";

primitive!(featurep, args, _env, ctx, {
    exact_arity(args, 1)?;
    let feature = args::name(args.first())?;
    Ok(ELispExp::boolean(
        ctx.loader(|loader| loader.is_loaded(&feature)),
    ))
});

pub const FEATURES_DOC: &str = "(features): Return the features loaded, as symbols, the most \
         recent first.\n\n\
         Example:\n\
         (memq 'dired (features))";

primitive!(features, args, _env, ctx, {
    exact_arity(args, 0)?;
    Ok(ELispExp::proper_list(
        ctx.loader(|loader| loader.loaded())
            .into_iter()
            .map(ELispExp::symbol)
            .collect(),
    ))
});

pub const FEATURE_FILE_DOC: &str = "(feature-file FEATURE): Return the file FEATURE was loaded \
         from, or nil when it is not loaded or came from no file.\n\n\
         Example:\n\
         (feature-file 'dired)";

primitive!(feature_file, args, _env, ctx, {
    exact_arity(args, 1)?;
    let feature = args::name(args.first())?;
    Ok(ctx
        .loader(|loader| {
            loader
                .feature(&feature)
                .filter(|known| known.state == FeatureState::Loaded)
                .and_then(|known| known.file.clone())
        })
        .map_or_else(ELispExp::nil, |file| {
            ELispExp::string(file.display().to_string())
        }))
});

pub const FEATURE_STATUS_DOC: &str = "(feature-status FEATURE): Return how FEATURE's load went: \
         loaded, loading while its file is being evaluated, failed when a `require' of it raised \
         an error, or nil when nothing has asked for it. The log says why one failed.\n\n\
         Example:\n\
         (feature-status 'vim) => loaded";

primitive!(feature_status, args, _env, ctx, {
    exact_arity(args, 1)?;
    let feature = args::name(args.first())?;
    let status = ctx.loader(|loader| {
        if loader.is_requiring(&feature) {
            return Some("loading");
        }
        loader.feature(&feature).map(|known| match known.state {
            FeatureState::Loaded => "loaded",
            FeatureState::Failed(_) => "failed",
        })
    });
    Ok(status.map_or_else(ELispExp::nil, |status| ELispExp::symbol(status.to_string())))
});

pub const EVAL_AFTER_LOAD_DOC: &str = "(eval-after-load FEATURE FUNCTION): Call FUNCTION, with \
         no arguments, once FEATURE is provided, and return nil; when it is already, call it now \
         and return its value. One that fails later is logged. `with-eval-after-load' is the \
         same with a body in place of the function.\n\n\
         Example:\n\
         (eval-after-load 'dired (lambda () (setq dired-sort 'type)))";

primitive!(eval_after_load, args, env, ctx, {
    exact_arity(args, 2)?;
    let feature = args::name(args.first())?;
    ctx.eval_after_load(&feature, args[1].clone(), &env)
});

pub const PLUGINS_DOC: &str = "(plugins): Return the plugins found at start, by name, as \
         symbols. Finding one adds its directory to `lisp-path' and loads nothing: `require' \
         loads it. `plugin-info' tells what one says of itself.\n\n\
         Example:\n\
         (plugins)";

primitive!(plugins, args, _env, ctx, {
    exact_arity(args, 0)?;
    Ok(ELispExp::proper_list(ctx.loader(|loader| {
        loader
            .plugins()
            .iter()
            .map(|plugin| ELispExp::symbol(plugin.name.clone()))
            .collect()
    })))
});

pub const PLUGIN_INFO_DOC: &str = "(plugin-info NAME): Return what the plugin NAME says of \
         itself in its plugin.lisp, as an association list -- version, description, requires \
         and directory -- or nil when no plugin of that name was found.\n\n\
         Example:\n\
         (cdr (assoc 'version (plugin-info 'my-plugin)))";

primitive!(plugin_info, args, _env, ctx, {
    exact_arity(args, 1)?;
    let name = args::name(args.first())?;
    let Some(plugin) = ctx.loader(|loader| {
        loader
            .plugins()
            .iter()
            .find(|plugin| plugin.name == name)
            .cloned()
    }) else {
        return Ok(ELispExp::nil());
    };
    let text = |text: Option<String>| text.map_or_else(ELispExp::nil, ELispExp::string);
    let entry = |key: &str, value| ELispExp::cons(ELispExp::symbol(key.into()), value);
    Ok(ELispExp::proper_list(vec![
        entry("version", text(plugin.version)),
        entry("description", text(plugin.description)),
        entry(
            "requires",
            ELispExp::proper_list(plugin.requires.into_iter().map(ELispExp::symbol).collect()),
        ),
        entry(
            "directory",
            ELispExp::string(plugin.directory.display().to_string()),
        ),
    ]))
});

/// Registers the primitives of loading.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("eval-after-load", eval_after_load, EVAL_AFTER_LOAD_DOC);
    into.function("feature-file", feature_file, FEATURE_FILE_DOC);
    into.function("feature-status", feature_status, FEATURE_STATUS_DOC);
    into.function("featurep", featurep, FEATUREP_DOC);
    into.function("features", features, FEATURES_DOC);
    into.function("plugin-info", plugin_info, PLUGIN_INFO_DOC);
    into.function("plugins", plugins, PLUGINS_DOC);
    into.function("provide", provide, PROVIDE_DOC);
    into.function("require", require, REQUIRE_DOC);
}
