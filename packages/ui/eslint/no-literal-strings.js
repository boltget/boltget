// ESLint rule `boltget/no-literal-strings`: user-visible text must come from the
// string catalog (DESIGN-PRINCIPLES.md, "Copy and strings"; TECH-STACK.md,
// ADR-015). Used by packages/ui and apps/desktop.
//
// It reports text that reaches the screen or a screen reader as a literal:
//
// - Svelte markup: text nodes (`<p>Done</p>`), and string literals in a text
//   mustache (`{busy ? "Saving" : label}`).
// - Svelte attributes and component props whose name says they are shown:
//   `aria-label`, `title`, `placeholder`, `alt`, `label`, ... (see
//   VISIBLE_ATTRIBUTES), as plain values or as literals in a mustache.
// - Script code (TS and `<script>`): string literals assigned to DOM text
//   (`el.textContent = "..."`, `el.title = ...`), passed to
//   `setAttribute("aria-label", "...")`, or given as the value of an object
//   property with a visible name (`{ label: "Pause" }`).
//
// The allowlist is narrow on purpose:
//
// - Text without a letter in it (`·`, `/`, `+`, `…`, `2×`, digits): symbols
//   and numbers are not words to translate.
// - Catalog keys: a literal of the form `a.b` or `a.b_c` (lower case, dotted,
//   no spaces) is a message key, not text, e.g. `{ label: "command.add" }`.
// - Anything else needs an `eslint-disable-next-line boltget/no-literal-strings
//   -- <reason>` comment at the site, so each exception is visible in review.

/** Attributes and props whose value is shown or announced. */
const VISIBLE_ATTRIBUTES = new Set([
  "aria-label",
  "aria-description",
  "aria-placeholder",
  "aria-roledescription",
  "aria-valuetext",
  "alt",
  "title",
  "placeholder",
  "label",
  "text",
  "description",
  "message",
  "tooltip",
  "heading",
  "hint",
  "caption",
]);

/** Object properties whose string value is shown (camelCase forms). */
const VISIBLE_PROPERTIES = new Set([
  "ariaLabel",
  "ariaDescription",
  "ariaPlaceholder",
  "ariaRoleDescription",
  "ariaValueText",
  "alt",
  "title",
  "placeholder",
  "label",
  "text",
  "description",
  "message",
  "tooltip",
  "heading",
  "hint",
  "caption",
  "body",
  "confirm",
]);

/** DOM properties that put text on the screen when assigned. */
const DOM_TEXT = new Set([
  "textContent",
  "innerText",
  "innerHTML",
  "title",
  "placeholder",
  "alt",
  "ariaLabel",
  "ariaDescription",
  "ariaValueText",
]);

const LETTER = /\p{L}/u;
const MESSAGE_KEY = /^[a-z][a-z0-9_]*(\.[a-z0-9_]+)+$/;

/** Whether a piece of literal text would be a word on screen. */
export function isVisibleText(text) {
  const trimmed = text.trim();
  if (!LETTER.test(trimmed)) return false;
  return !MESSAGE_KEY.test(trimmed);
}

/** The literal strings an expression can evaluate to, directly. */
function literalsOf(node, out = []) {
  if (!node) return out;
  switch (node.type) {
    case "Literal":
      if (typeof node.value === "string") out.push(node);
      break;
    case "TemplateLiteral":
      if (node.quasis.some((q) => isVisibleText(q.value.cooked ?? q.value.raw))) out.push(node);
      break;
    case "ConditionalExpression":
      literalsOf(node.consequent, out);
      literalsOf(node.alternate, out);
      break;
    case "LogicalExpression":
      literalsOf(node.left, out);
      literalsOf(node.right, out);
      break;
    case "BinaryExpression":
      if (node.operator === "+") {
        literalsOf(node.left, out);
        literalsOf(node.right, out);
      }
      break;
    case "TSAsExpression":
    case "TSSatisfiesExpression":
    case "TSNonNullExpression":
      literalsOf(node.expression, out);
      break;
    default:
      break;
  }
  return out;
}

function textOf(node) {
  if (node.type === "Literal") return String(node.value);
  if (node.type === "TemplateLiteral") return node.quasis.map((q) => q.value.cooked ?? "").join("");
  return "";
}

function propertyName(key) {
  if (!key) return null;
  if (key.type === "Identifier") return key.name;
  if (key.type === "Literal" && typeof key.value === "string") return key.value;
  return null;
}

/** Whether a node sits inside an element whose text is not shown. */
function insideHiddenElement(node) {
  for (let p = node.parent; p; p = p.parent) {
    if (p.type === "SvelteScriptElement" || p.type === "SvelteStyleElement") return true;
  }
  return false;
}

/** @type {import("eslint").Rule.RuleModule} */
export const noLiteralStrings = {
  meta: {
    type: "problem",
    docs: {
      description: "Require user-visible text to come from the string catalog (t()), not literals.",
    },
    schema: [],
    messages: {
      literal:
        'User-visible text "{{text}}" is a literal. Add it to packages/ui/src/messages/en.json and use t().',
    },
  },
  create(context) {
    function report(node, text) {
      const shown = text.trim().replace(/\s+/g, " ");
      context.report({
        node,
        messageId: "literal",
        data: { text: shown.length > 40 ? `${shown.slice(0, 39)}…` : shown },
      });
    }

    function checkExpression(expression) {
      for (const literal of literalsOf(expression)) {
        const text = textOf(literal);
        if (isVisibleText(text)) report(literal, text);
      }
    }

    return {
      // <p>Done</p>
      SvelteText(node) {
        if (insideHiddenElement(node)) return;
        if (isVisibleText(node.value)) report(node, node.value);
      },
      // <p>{busy ? "Saving" : label}</p>
      SvelteMustacheTag(node) {
        if (node.parent?.type === "SvelteAttribute") return;
        checkExpression(node.expression);
      },
      // aria-label="Close", title={x ? "On" : "Off"}
      SvelteAttribute(node) {
        const name = node.key?.name;
        if (typeof name !== "string" || !VISIBLE_ATTRIBUTES.has(name)) return;
        for (const part of node.value ?? []) {
          if (part.type === "SvelteLiteral") {
            if (isVisibleText(part.value)) report(part, part.value);
          } else if (part.type === "SvelteMustacheTag") {
            checkExpression(part.expression);
          }
        }
      },
      // { label: "Pause" }
      Property(node) {
        if (node.parent?.type !== "ObjectExpression") return;
        const name = propertyName(node.key);
        if (!name || !VISIBLE_PROPERTIES.has(name)) return;
        checkExpression(node.value);
      },
      // el.textContent = "Done"
      AssignmentExpression(node) {
        const target = node.left;
        if (target.type !== "MemberExpression") return;
        const name = propertyName(target.property);
        if (!name || !DOM_TEXT.has(name)) return;
        checkExpression(node.right);
      },
      // el.setAttribute("aria-label", "Close")
      CallExpression(node) {
        const callee = node.callee;
        if (callee.type !== "MemberExpression" || propertyName(callee.property) !== "setAttribute")
          return;
        const [attribute, value] = node.arguments;
        if (attribute?.type !== "Literal" || !VISIBLE_ATTRIBUTES.has(String(attribute.value)))
          return;
        checkExpression(value);
      },
    };
  },
};

/** The plugin, for `plugins: { boltget }` in a flat config. */
export const plugin = {
  meta: { name: "boltget" },
  rules: { "no-literal-strings": noLiteralStrings },
};

/**
 * A flat-config block that turns the rule on for UI sources. Tests are not
 * UI, and `ignores` takes the few files whose text is not the app's own: the
 * development-only simulated engine, whose strings stand in for the engine's
 * English diagnostics (shown under "Technical details" by design,
 * ARCHITECTURE.md, "Errors").
 */
export function catalogRule(files, ignores = []) {
  return {
    files,
    ignores: ["**/*.test.ts", "**/*.test.js", ...ignores],
    plugins: { boltget: plugin },
    rules: { "boltget/no-literal-strings": "error" },
  };
}
