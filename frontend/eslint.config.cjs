const typescriptEslint = require("@typescript-eslint/eslint-plugin");

module.exports = [
  { ignores: ["dist"] },
  ...typescriptEslint.configs["flat/recommended"],
];
