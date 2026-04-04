pub use octocrab::Octocrab;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repo {
    pub owner: String,
    pub name: String,
    pub description: Option<String>,
    pub url: String,
    pub language: Option<String>,
    pub stars: u32,
    pub visibility: String,
    pub last_updated: String,
    pub readme: Option<String>,
}

#[derive(Debug, Error)]
pub enum RepoKaiError {
    #[error("no GitHub token found (set GITHUB_TOKEN or log in with `gh auth login`)")]
    MissingToken,
    #[error("GitHub API error: {0}")]
    GitHub(#[from] octocrab::Error),
    #[error("base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("UTF-8 decode error: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("git error: {0}")]
    Git(String),
    #[error("path error: {0}")]
    Path(String),
}

fn resolve_token() -> Result<String, RepoKaiError> {
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        return Ok(token);
    }
    // Fall back to `gh auth token`
    let output = std::process::Command::new("gh")
        .args(["auth", "token"])
        .output()
        .map_err(|_| RepoKaiError::MissingToken)?;
    if output.status.success() {
        let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !token.is_empty() {
            return Ok(token);
        }
    }
    Err(RepoKaiError::MissingToken)
}

pub async fn create_client() -> Result<Octocrab, RepoKaiError> {
    let token = resolve_token()?;
    Ok(Octocrab::builder().personal_token(token).build()?)
}

pub async fn get_authenticated_user(client: &Octocrab) -> Result<String, RepoKaiError> {
    let user = client.current().user().await?;
    Ok(user.login)
}

pub async fn fetch_repos(client: &Octocrab) -> Result<Vec<Repo>, RepoKaiError> {
    let mut all_repos = Vec::new();
    let mut page_num = 1u8;

    loop {
        let page = client
            .current()
            .list_repos_for_authenticated_user()
            .sort("updated")
            .per_page(100)
            .page(page_num)
            .send()
            .await?;

        if page.items.is_empty() {
            break;
        }

        for repo in &page.items {
            let owner = repo
                .owner
                .as_ref()
                .map(|o| o.login.clone())
                .unwrap_or_default();

            all_repos.push(Repo {
                owner,
                name: repo.name.clone(),
                description: repo.description.clone(),
                url: repo
                    .html_url
                    .as_ref()
                    .map(|u| u.to_string())
                    .unwrap_or_default(),
                language: repo.language.as_ref().and_then(|v| v.as_str()).map(String::from),
                stars: repo.stargazers_count.unwrap_or(0) as u32,
                visibility: if repo.private.unwrap_or(false) {
                    "private".into()
                } else {
                    "public".into()
                },
                last_updated: repo
                    .updated_at
                    .map(|dt| dt.to_string())
                    .unwrap_or_default(),
                readme: None,
            });
        }

        if page.next.is_none() {
            break;
        }
        page_num += 1;
    }

    Ok(all_repos)
}

#[derive(Deserialize)]
struct ReadmeResponse {
    content: Option<String>,
}

pub async fn fetch_readme(
    client: &Octocrab,
    owner: &str,
    repo: &str,
) -> Result<Option<String>, RepoKaiError> {
    let response: Result<ReadmeResponse, _> = client
        .get(format!("/repos/{owner}/{repo}/readme"), None::<&()>)
        .await;

    match response {
        Ok(readme) => {
            if let Some(encoded) = readme.content {
                let cleaned: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();
                use base64::Engine;
                let bytes = base64::engine::general_purpose::STANDARD.decode(cleaned)?;
                Ok(Some(String::from_utf8(bytes)?))
            } else {
                Ok(None)
            }
        }
        Err(_) => Ok(None),
    }
}

// ---- Publish local repo to GitHub ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishOptions {
    pub local_path: String,
    pub name: String,
    pub description: String,
    pub private: bool,
}

pub async fn publish_local_repo(
    client: &Octocrab,
    opts: &PublishOptions,
) -> Result<Repo, RepoKaiError> {
    let path = Path::new(&opts.local_path);

    // Verify it's a git repo
    if !path.join(".git").exists() {
        return Err(RepoKaiError::Path(format!(
            "{} is not a git repository",
            opts.local_path
        )));
    }

    // Check if origin remote already exists
    let remote_check = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(path)
        .output()
        .map_err(|e| RepoKaiError::Git(e.to_string()))?;

    if remote_check.status.success() {
        let existing = String::from_utf8_lossy(&remote_check.stdout).trim().to_string();
        return Err(RepoKaiError::Git(format!(
            "origin remote already exists: {existing}"
        )));
    }

    // Create empty repo on GitHub (no auto-init)
    let repo = client
        .post(
            "/user/repos",
            Some(&serde_json::json!({
                "name": opts.name,
                "description": opts.description,
                "private": opts.private,
                "auto_init": false,
            })),
        )
        .await
        .map_err(|e| RepoKaiError::GitHub(e))?;

    let repo: octocrab::models::Repository = repo;
    let clone_url = repo
        .clone_url
        .as_ref()
        .map(|u| u.to_string())
        .unwrap_or_default();

    // Add origin remote
    let add_remote = Command::new("git")
        .args(["remote", "add", "origin", &clone_url])
        .current_dir(path)
        .output()
        .map_err(|e| RepoKaiError::Git(e.to_string()))?;

    if !add_remote.status.success() {
        let err = String::from_utf8_lossy(&add_remote.stderr).to_string();
        return Err(RepoKaiError::Git(format!("failed to add remote: {err}")));
    }

    // Push all branches
    let push = Command::new("git")
        .args(["push", "-u", "origin", "--all"])
        .current_dir(path)
        .output()
        .map_err(|e| RepoKaiError::Git(e.to_string()))?;

    if !push.status.success() {
        let err = String::from_utf8_lossy(&push.stderr).to_string();
        return Err(RepoKaiError::Git(format!("failed to push: {err}")));
    }

    let owner = repo
        .owner
        .as_ref()
        .map(|o| o.login.clone())
        .unwrap_or_default();

    Ok(Repo {
        owner,
        name: repo.name.clone(),
        description: repo.description.clone(),
        url: repo
            .html_url
            .as_ref()
            .map(|u| u.to_string())
            .unwrap_or_default(),
        language: repo.language.as_ref().and_then(|v| v.as_str()).map(String::from),
        stars: 0,
        visibility: if opts.private { "private".into() } else { "public".into() },
        last_updated: repo
            .updated_at
            .map(|dt| dt.to_string())
            .unwrap_or_default(),
        readme: None,
    })
}

// ---- Clone repo locally ----

pub fn clone_repo(url: &str, destination: &str) -> Result<(), RepoKaiError> {
    let dest = Path::new(destination);
    if dest.exists() && dest.read_dir().map(|mut d| d.next().is_some()).unwrap_or(false) {
        return Err(RepoKaiError::Path(format!(
            "{destination} already exists and is not empty"
        )));
    }

    let output = Command::new("git")
        .args(["clone", url, destination])
        .output()
        .map_err(|e| RepoKaiError::Git(e.to_string()))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(RepoKaiError::Git(format!("clone failed: {err}")));
    }

    Ok(())
}

// ---- Update repo settings ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateRepoOptions {
    pub description: Option<String>,
    pub private: Option<bool>,
}

pub async fn update_repo(
    client: &Octocrab,
    owner: &str,
    repo: &str,
    opts: &UpdateRepoOptions,
) -> Result<(), RepoKaiError> {
    let mut body = serde_json::Map::new();
    if let Some(desc) = &opts.description {
        body.insert("description".into(), serde_json::json!(desc));
    }
    if let Some(private) = opts.private {
        body.insert("private".into(), serde_json::json!(private));
    }

    let _: serde_json::Value = client
        .patch(
            format!("/repos/{owner}/{repo}"),
            Some(&serde_json::Value::Object(body)),
        )
        .await?;

    Ok(())
}
