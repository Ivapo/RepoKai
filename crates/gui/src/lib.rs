use repokai_core::{PublishOptions, Repo, UpdateRepoOptions};

#[tauri::command]
async fn get_user() -> Result<String, String> {
    let client = repokai_core::create_client()
        .await
        .map_err(|e| e.to_string())?;
    repokai_core::get_authenticated_user(&client)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_repos() -> Result<Vec<Repo>, String> {
    let client = repokai_core::create_client()
        .await
        .map_err(|e| e.to_string())?;
    repokai_core::fetch_repos(&client)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_starred_repos() -> Result<Vec<Repo>, String> {
    let client = repokai_core::create_client()
        .await
        .map_err(|e| e.to_string())?;
    repokai_core::fetch_starred_repos(&client)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_readme(owner: String, repo: String) -> Result<Option<String>, String> {
    let client = repokai_core::create_client()
        .await
        .map_err(|e| e.to_string())?;
    repokai_core::fetch_readme(&client, &owner, &repo)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn publish_repo(
    local_path: String,
    name: String,
    description: String,
    private: bool,
) -> Result<Repo, String> {
    let client = repokai_core::create_client()
        .await
        .map_err(|e| e.to_string())?;
    let opts = PublishOptions {
        local_path,
        name,
        description,
        private,
    };
    repokai_core::publish_local_repo(&client, &opts)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn clone_repo(url: String, destination: String) -> Result<(), String> {
    repokai_core::clone_repo(&url, &destination).map_err(|e| e.to_string())
}

#[tauri::command]
async fn update_repo(
    owner: String,
    repo: String,
    description: Option<String>,
    private: Option<bool>,
) -> Result<(), String> {
    let client = repokai_core::create_client()
        .await
        .map_err(|e| e.to_string())?;
    let opts = UpdateRepoOptions {
        description,
        private,
    };
    repokai_core::update_repo(&client, &owner, &repo, &opts)
        .await
        .map_err(|e| e.to_string())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            get_user,
            get_repos,
            get_starred_repos,
            get_readme,
            publish_repo,
            clone_repo,
            update_repo,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
