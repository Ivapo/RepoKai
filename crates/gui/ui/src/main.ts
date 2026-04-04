import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { marked } from "marked";
import "./style.css";

interface Repo {
  owner: string;
  name: string;
  description: string | null;
  url: string;
  language: string | null;
  stars: number;
  visibility: string;
  last_updated: string;
  readme: string | null;
}

type ThemeName = "dark" | "light" | "3.1" | "tui";
const THEMES: ThemeName[] = ["dark", "light", "3.1", "tui"];
const STORAGE_KEY = "repokai_theme";

let repos: Repo[] = [];
let selectedIndex = -1;
let dialogOpen = false;
let readmeRaw = "";
let readmeRendered = false;
let sortOrder: "recent" | "a-z" = "recent";
let reposOriginal: Repo[] = [];

// ---- Theme cycling ----

function getTheme(): ThemeName {
  const stored = localStorage.getItem(STORAGE_KEY);
  return THEMES.includes(stored as ThemeName) ? (stored as ThemeName) : "dark";
}

function setTheme(theme: ThemeName): void {
  localStorage.setItem(STORAGE_KEY, theme);
  if (theme === "dark") {
    document.documentElement.removeAttribute("data-theme");
  } else {
    document.documentElement.setAttribute("data-theme", theme);
  }
  document.getElementById("theme-toggle")!.textContent = theme;
}

function cycleTheme(): void {
  const current = getTheme();
  const next = THEMES[(THEMES.indexOf(current) + 1) % THEMES.length]!;
  setTheme(next);
}

setTheme(getTheme());
document.getElementById("theme-toggle")!.addEventListener("click", cycleTheme);

// ---- Dialog helpers ----

function showDialog(title: string, bodyHtml: string, actions: { label: string; cls?: string; onClick: () => void }[]): void {
  dialogOpen = true;
  document.getElementById("dialog-title")!.textContent = title;
  document.getElementById("dialog-body")!.innerHTML = bodyHtml;
  const actionsEl = document.getElementById("dialog-actions")!;
  actionsEl.innerHTML = "";
  for (const action of actions) {
    const btn = document.createElement("button");
    btn.textContent = action.label;
    btn.className = "dialog-btn" + (action.cls ? ` ${action.cls}` : "");
    btn.addEventListener("click", action.onClick);
    actionsEl.appendChild(btn);
  }
  document.getElementById("dialog-overlay")!.classList.remove("hidden");
  // Focus the first input if any
  const firstInput = document.querySelector<HTMLInputElement>("#dialog-body input, #dialog-body select");
  if (firstInput) firstInput.focus();
}

function closeDialog(): void {
  dialogOpen = false;
  document.getElementById("dialog-overlay")!.classList.add("hidden");
}

function getDialogInput(name: string): string {
  return (document.querySelector<HTMLInputElement | HTMLTextAreaElement>(`#dialog-body [name="${name}"]`)?.value ?? "").trim();
}

function getDialogChecked(name: string): boolean {
  return document.querySelector<HTMLInputElement>(`#dialog-body [name="${name}"]`)?.checked ?? false;
}

// ---- App ----

function applySort(): void {
  if (sortOrder === "a-z") {
    repos = [...reposOriginal].sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()));
  } else {
    repos = [...reposOriginal];
  }
  document.getElementById("btn-sort")!.textContent = sortOrder;
}

function toggleSort(): void {
  sortOrder = sortOrder === "recent" ? "a-z" : "recent";
  applySort();
  renderRepoList();
  if (repos.length > 0 && selectedIndex >= 0) {
    selectRepo(Math.min(selectedIndex, repos.length - 1));
  }
}

async function init(): Promise<void> {
  try {
    const username = await invoke<string>("get_user");
    document.getElementById("user-info")!.textContent = username;
  } catch {
    document.getElementById("user-info")!.textContent = "not logged in";
  }

  try {
    reposOriginal = await invoke<Repo[]>("get_repos");
    applySort();
    renderRepoList();
    if (repos.length > 0) {
      selectRepo(0);
    } else {
      document.getElementById("readme-content")!.innerHTML =
        '<div class="loading">No repositories found</div>';
    }
  } catch (err) {
    document.getElementById("readme-content")!.innerHTML =
      '<div class="loading">Error: ' + escapeHtml(String(err)) + "</div>";
  }
}

async function refreshRepos(): Promise<void> {
  try {
    reposOriginal = await invoke<Repo[]>("get_repos");
    applySort();
    renderRepoList();
    if (selectedIndex >= repos.length) selectedIndex = repos.length - 1;
    if (selectedIndex >= 0) selectRepo(selectedIndex);
  } catch {
    // keep current state
  }
}

function renderRepoList(): void {
  const ul = document.getElementById("repos")!;
  ul.innerHTML = "";

  repos.forEach((repo, i) => {
    const li = document.createElement("li");
    if (i === selectedIndex) li.classList.add("selected");

    const name = document.createElement("span");
    name.className = "repo-name";
    name.textContent = repo.name;

    const vis = document.createElement("span");
    vis.className = "repo-visibility";
    vis.textContent = repo.visibility;

    li.appendChild(name);
    if (repo.language) {
      const lang = document.createElement("span");
      lang.className = "repo-lang";
      lang.textContent = repo.language;
      li.appendChild(lang);
    }
    li.appendChild(vis);

    li.addEventListener("click", () => selectRepo(i));
    ul.appendChild(li);
  });
}

async function selectRepo(index: number): Promise<void> {
  selectedIndex = index;
  renderRepoList();

  const repo = repos[index];
  if (!repo) return;

  renderInfo(repo);

  const readmeEl = document.getElementById("readme-content")!;
  readmeEl.innerHTML = '<div class="loading">Loading README\u2026</div>';
  readmeEl.classList.remove("rendered");

  try {
    const readme = await invoke<string | null>("get_readme", {
      owner: repo.owner,
      repo: repo.name,
    });
    readmeRaw = readme || "";
    displayReadme();
  } catch {
    readmeRaw = "";
    readmeEl.textContent = "Could not load README";
  }
}

function renderInfo(repo: Repo): void {
  const el = document.getElementById("info-content")!;
  const desc = repo.description || "No description";
  const lang = repo.language || "Unknown";

  el.innerHTML =
    row("Name", escapeHtml(repo.name)) +
    row("Description", escapeHtml(desc)) +
    row("URL", `<a href="${escapeHtml(repo.url)}">${escapeHtml(repo.url)}</a>`) +
    row("Language", escapeHtml(lang), "lang") +
    row("Stars", "\u2605 " + repo.stars, "stars") +
    row("Visibility", escapeHtml(repo.visibility)) +
    row("Updated", escapeHtml(repo.last_updated));
}

function row(label: string, value: string, cls?: string): string {
  const extra = cls ? ` ${cls}` : "";
  return `<div class="info-row"><span class="info-label">${label}</span><span class="info-value${extra}">${value}</span></div>`;
}

function escapeHtml(str: string): string {
  const el = document.createElement("span");
  el.textContent = str;
  return el.innerHTML;
}

function scrollToSelected(): void {
  const item = document.querySelector("#repos li.selected");
  if (item) item.scrollIntoView({ block: "nearest" });
}

function displayReadme(): void {
  const el = document.getElementById("readme-content")!;
  const toggle = document.getElementById("readme-toggle")!;

  if (!readmeRaw) {
    el.textContent = "No README found";
    el.classList.remove("rendered");
    return;
  }

  if (readmeRendered) {
    el.innerHTML = marked.parse(readmeRaw) as string;
    el.classList.add("rendered");
    toggle.textContent = "Raw";
    toggle.classList.add("active");
  } else {
    el.textContent = readmeRaw;
    el.classList.remove("rendered");
    toggle.textContent = "HTML";
    toggle.classList.remove("active");
  }
}

document.getElementById("readme-toggle")!.addEventListener("click", () => {
  readmeRendered = !readmeRendered;
  displayReadme();
});

// ---- Publish dialog ----

async function showPublishDialog(): Promise<void> {
  const folder = await openDialog({ directory: true, title: "Select local repo to publish" });
  if (!folder) return;

  const folderPath = typeof folder === "string" ? folder : folder;
  const defaultName = folderPath.split("/").pop() ?? "";

  showDialog(
    "Publish Local Repo to GitHub",
    `<label>Local path<input name="local_path" type="text" value="${escapeHtml(folderPath)}" readonly /></label>
     <label>Repo name<input name="name" type="text" value="${escapeHtml(defaultName)}" placeholder="my-project" /></label>
     <label>Description<textarea name="description" rows="3" placeholder="Optional description"></textarea></label>
     <label class="checkbox"><input name="private" type="checkbox" /> Private repository</label>`,
    [
      { label: "Cancel", onClick: closeDialog },
      {
        label: "Publish",
        cls: "primary",
        onClick: async () => {
          const localPath = getDialogInput("local_path");
          const name = getDialogInput("name");
          const description = getDialogInput("description");
          const priv_ = getDialogChecked("private");

          if (!localPath || !name) return;

          const btn = document.querySelector<HTMLButtonElement>(".dialog-btn.primary")!;
          btn.textContent = "Publishing\u2026";
          btn.disabled = true;

          try {
            await invoke("publish_repo", {
              localPath,
              name,
              description,
              private: priv_,
            });
            closeDialog();
            await refreshRepos();
          } catch (err) {
            btn.textContent = "Publish";
            btn.disabled = false;
            alert(String(err));
          }
        },
      },
    ],
  );
}

// ---- Clone dialog ----

async function showCloneDialog(): Promise<void> {
  const repo = repos[selectedIndex];
  if (!repo) return;

  const folder = await openDialog({ directory: true, title: `Select destination for ${repo.name}` });
  if (!folder) return;

  const destination = (typeof folder === "string" ? folder : folder) + "/" + repo.name;

  showDialog(
    `Clone ${repo.name}`,
    `<label>Destination<input name="destination" type="text" value="${escapeHtml(destination)}" /></label>`,
    [
      { label: "Cancel", onClick: closeDialog },
      {
        label: "Clone",
        cls: "primary",
        onClick: async () => {
          const dest = getDialogInput("destination");
          if (!dest) return;

          const btn = document.querySelector<HTMLButtonElement>(".dialog-btn.primary")!;
          btn.textContent = "Cloning\u2026";
          btn.disabled = true;

          try {
            await invoke("clone_repo", { url: repo.url + ".git", destination: dest });
            closeDialog();
            alert(`Cloned to ${dest}`);
          } catch (err) {
            btn.textContent = "Clone";
            btn.disabled = false;
            alert(String(err));
          }
        },
      },
    ],
  );
}

// ---- Edit dialog ----

function showEditDialog(): void {
  const repo = repos[selectedIndex];
  if (!repo) return;

  const isPrivate = repo.visibility === "private";
  showDialog(
    `Edit ${repo.name}`,
    `<label>Description<textarea name="description" rows="3">${escapeHtml(repo.description || "")}</textarea></label>
     <label>Visibility<select name="visibility">
       <option value="public" ${!isPrivate ? "selected" : ""}>Public</option>
       <option value="private" ${isPrivate ? "selected" : ""}>Private</option>
     </select></label>`,
    [
      { label: "Cancel", onClick: closeDialog },
      {
        label: "Save",
        cls: "primary",
        onClick: async () => {
          const description = getDialogInput("description");
          const visibility = (document.querySelector<HTMLSelectElement>('#dialog-body [name="visibility"]')?.value) ?? "public";
          const priv_ = visibility === "private";

          const btn = document.querySelector<HTMLButtonElement>(".dialog-btn.primary")!;
          btn.textContent = "Saving\u2026";
          btn.disabled = true;

          try {
            await invoke("update_repo", {
              owner: repo.owner,
              repo: repo.name,
              description,
              private: priv_,
            });
            closeDialog();
            await refreshRepos();
          } catch (err) {
            btn.textContent = "Save";
            btn.disabled = false;
            alert(String(err));
          }
        },
      },
    ],
  );
}

// ---- Toolbar buttons ----

document.getElementById("btn-sort")!.addEventListener("click", toggleSort);
document.getElementById("btn-refresh")!.addEventListener("click", refreshRepos);
document.getElementById("btn-publish")!.addEventListener("click", showPublishDialog);
document.getElementById("btn-clone")!.addEventListener("click", showCloneDialog);
document.getElementById("btn-edit")!.addEventListener("click", showEditDialog);

// Close dialog on overlay click or Escape
document.getElementById("dialog-overlay")!.addEventListener("click", (e) => {
  if (e.target === document.getElementById("dialog-overlay")) closeDialog();
});

// ---- Keyboard nav ----

document.addEventListener("keydown", (e: KeyboardEvent) => {
  if (dialogOpen) {
    if (e.key === "Escape") closeDialog();
    return;
  }

  if (e.key === "ArrowUp") {
    e.preventDefault();
    if (selectedIndex > 0) {
      selectRepo(selectedIndex - 1);
      scrollToSelected();
    }
  } else if (e.key === "ArrowDown") {
    e.preventDefault();
    if (selectedIndex < repos.length - 1) {
      selectRepo(selectedIndex + 1);
      scrollToSelected();
    }
  } else if (e.key === "t") {
    cycleTheme();
  } else if (e.key === "p") {
    showPublishDialog();
  } else if (e.key === "c") {
    showCloneDialog();
  } else if (e.key === "e") {
    showEditDialog();
  } else if (e.key === "s") {
    toggleSort();
  } else if (e.key === "r") {
    refreshRepos();
  } else if (e.key === "o") {
    const repo = repos[selectedIndex];
    if (repo?.url) openUrl(repo.url);
  }
});

// Open all links in external browser
document.addEventListener("click", (e: MouseEvent) => {
  const anchor = (e.target as HTMLElement).closest("a");
  if (anchor?.href) {
    e.preventDefault();
    openUrl(anchor.href);
  }
});

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", init);
} else {
  init();
}
