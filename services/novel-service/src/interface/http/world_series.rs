use super::*;
use crate::application::world_series::{CreateWorldSeries, WorldSeriesApplicationError};

pub(super) fn private(response: Response) -> Response {
    let mut response = response;
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    response
}

pub(super) fn error(error: WorldSeriesApplicationError) -> Response {
    let (status, code, message) = match error {
        WorldSeriesApplicationError::InvalidInput => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_world_series", "Invalid world series input"),
        WorldSeriesApplicationError::NotFound => (StatusCode::NOT_FOUND, "not_found", "Novel or series not found"),
        WorldSeriesApplicationError::SourceUnavailable => (StatusCode::CONFLICT, "series_rule_source_unavailable", "Generate ready advanced rules in the source novel before creating a series"),
        WorldSeriesApplicationError::SourceAlreadyAssociated => (StatusCode::CONFLICT, "source_already_in_series", "Source novel already belongs to a series; select that series or explicitly clear its association"),
        WorldSeriesApplicationError::NovelNotReady => (StatusCode::CONFLICT, "canon_unavailable", "Novel is not ready"),
        WorldSeriesApplicationError::Repository(_) => (StatusCode::SERVICE_UNAVAILABLE, "series_unavailable", "World series is temporarily unavailable"),
    };
    private(coded_api_error(status, code, message))
}

pub(super) async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(command): Json<CreateWorldSeries>,
) -> Response {
    let Some(user_id) = extract_user_id(&headers) else {
        return private(api_error(StatusCode::UNAUTHORIZED, "Missing user ID"));
    };
    match state.series_handler.create(user_id, command).await {
        Ok(series) => private((StatusCode::CREATED, Json(series)).into_response()),
        Err(err) => error(err),
    }
}

pub(super) async fn list(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(user_id) = extract_user_id(&headers) else {
        return private(api_error(StatusCode::UNAUTHORIZED, "Missing user ID"));
    };
    match state.series_handler.list(user_id).await {
        Ok(series) => private(Json(series).into_response()),
        Err(err) => error(err),
    }
}

pub(super) async fn get_for_novel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(novel_id): Path<Uuid>,
) -> Response {
    let Some(user_id) = extract_user_id(&headers) else {
        return private(api_error(StatusCode::UNAUTHORIZED, "Missing user ID"));
    };
    match state.series_handler.get_for_novel(user_id, novel_id).await {
        Ok(series) => private(Json(series).into_response()),
        Err(err) => error(err),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Selection {
    #[serde(deserialize_with = "explicit_selection")]
    series_id: Option<Uuid>,
}

fn explicit_selection<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Uuid>, D::Error> {
    Option::<Uuid>::deserialize(deserializer)
}

pub(super) async fn associate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(novel_id): Path<Uuid>,
    Json(selection): Json<Selection>,
) -> Response {
    let Some(user_id) = extract_user_id(&headers) else {
        return private(api_error(StatusCode::UNAUTHORIZED, "Missing user ID"));
    };
    match state
        .series_handler
        .associate(user_id, novel_id, selection.series_id)
        .await
    {
        Ok(series) => private(Json(series).into_response()),
        Err(err) => error(err),
    }
}

pub(super) async fn suggestion(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(novel_id): Path<Uuid>,
) -> Response {
    let Some(user_id) = extract_user_id(&headers) else {
        return private(api_error(StatusCode::UNAUTHORIZED, "Missing user ID"));
    };
    match state.series_handler.suggest(user_id, novel_id).await {
        Ok(suggestion) => private(Json(suggestion).into_response()),
        Err(err) => error(err),
    }
}

#[cfg(test)]
mod tests {
    use super::Selection;

    #[test]
    fn clearing_a_series_requires_an_explicit_null_selection() {
        assert!(serde_json::from_str::<Selection>("{}").is_err());
        assert!(serde_json::from_str::<Selection>(r#"{"series_id":null}"#)
            .unwrap()
            .series_id
            .is_none());
        assert!(
            serde_json::from_str::<Selection>(r#"{"series_id":null,"user_id":"forged"}"#).is_err()
        );
    }
}
