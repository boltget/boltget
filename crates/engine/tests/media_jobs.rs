//! Media jobs through the public API: stored with the downloads, kept across
//! restarts.
#![allow(clippy::unwrap_used)]

use boltget_engine::{
    ContainerPreference, Engine, EngineConfig, Error, MediaJobId, MediaJobState, MediaJobUpdate,
    NewMediaJob,
};

#[tokio::test]
async fn media_jobs_are_stored_and_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::open(EngineConfig::default(), dir.path()).unwrap();
    let mut job = NewMediaJob::new("https://video.test/watch?v=abc", "bv*+ba/b", dir.path());
    job.preset = Some("best".into());
    job.title = Some("Clip".into());
    let added = engine.add_media_job(job).await.unwrap();
    assert_eq!(added.id, MediaJobId(1));
    assert_eq!(added.state, MediaJobState::Queued);
    assert_eq!(added.container, ContainerPreference::Mp4);

    let changed = engine
        .update_media_job(
            added.id,
            MediaJobUpdate {
                state: Some(MediaJobState::Downloading),
                ..MediaJobUpdate::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(changed.state, MediaJobState::Downloading);
    assert!(changed.updated_at >= added.updated_at);

    // Refused: not a web page, or no format.
    for bad in [
        NewMediaJob::new("file:///C:/x", "b", dir.path()),
        NewMediaJob::new("https://video.test/", " ", dir.path()),
    ] {
        assert!(engine.add_media_job(bad).await.is_err());
    }
    let missing = engine.media_job(MediaJobId(42)).await.unwrap_err();
    assert!(matches!(missing, Error::UnknownMediaJob(_)), "{missing}");
    assert_eq!(missing.message_key(), "error.unknown_media_job");
    engine.shutdown().await;

    let engine = Engine::open(EngineConfig::default(), dir.path()).unwrap();
    let jobs = engine.media_jobs().await.unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].title.as_deref(), Some("Clip"));
    assert_eq!(jobs[0].state, MediaJobState::Downloading);
    engine.remove_media_job(added.id).await.unwrap();
    assert!(engine.remove_media_job(added.id).await.is_err());
    assert_eq!(engine.media_jobs().await.unwrap().len(), 0);
    engine.shutdown().await;

    // Without a database there are none, and none can be added.
    let memory = Engine::new(EngineConfig::default()).unwrap();
    assert_eq!(memory.media_jobs().await.unwrap().len(), 0);
    assert!(
        memory
            .add_media_job(NewMediaJob::new("https://v.test/", "b", "/"))
            .await
            .is_err()
    );
}
