export function ensureDependenciesInstalled(): void {
  throw new Error(
    "Installed skills are immutable. Use the dependency-free native `pstack` command instead."
  );
}
