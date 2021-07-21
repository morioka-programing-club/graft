// Little Hack: Using `import` directly on neon::reflect::eval does not work for some reason.
// Here, we bridge any call to `import` through a `require`.
module.exports = specifier => import(specifier);