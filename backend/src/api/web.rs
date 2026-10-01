use super::*;
pub(crate) async fn web_search(req: Request, _user: AuthedUser) -> Result<JsonResp, ApiError> {
    let params = query(&req);
    let query = params
        .get("query")
        .cloned()
        .or_else(|| params.get("q").cloned())
        .ok_or_else(|| ApiError::bad_request("missing ?query= parameter"))?;
    let limit = params
        .get("limit")
        .cloned()
        .and_then(|l| l.parse::<usize>().ok())
        .unwrap_or(5);

    match crate::web::search_web_internal(&query, limit).await {
        Ok(results) => Ok(json_response(200, &results)),
        Err(e) => Err(ApiError::internal(format!("web search failed: {e}"))),
    }
}

pub(crate) async fn web_fetch(req: Request, _user: AuthedUser) -> Result<JsonResp, ApiError> {
    let params = query(&req);
    let url = params
        .get("url")
        .cloned()
        .ok_or_else(|| ApiError::bad_request("missing ?url= parameter"))?;

    match crate::web::fetch_page_internal(&url).await {
        Ok(content) => Ok(json_response(
            200,
            &serde_json::json!({
                "url": url,
                "content": content,
            }),
        )),
        Err(e) => Err(ApiError::bad_request(format!("web fetch failed: {e}"))),
    }
}
