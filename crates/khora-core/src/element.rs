use serde::{Deserialize, Serialize};

/// Information about a DOM element found via CSS selector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementInfo {
    /// CSS selector used to find this element.
    pub selector: String,
    /// Tag name (e.g., "div", "button", "input").
    pub tag_name: String,
    /// Inner text content, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Element attributes as key-value pairs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attributes: Option<serde_json::Value>,
    /// Bounding box in viewport coordinates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounding_box: Option<BoundingBox>,
    /// Whether the element is visible.
    #[serde(default)]
    pub visible: bool,
    /// Number of elements matching the selector.
    pub match_count: usize,
    /// Index of this element among matches (0-based).
    pub match_index: usize,
}

/// Bounding box of an element in viewport coordinates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundingBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A captured network request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkRequest {
    pub url: String,
    pub method: String,
    pub status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_type: Option<String>,
}

/// A console message from the browser.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleMessage {
    pub level: String,
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_element_info_roundtrip() {
        let el = ElementInfo {
            selector: "button.submit".to_string(),
            tag_name: "button".to_string(),
            text: Some("Submit".to_string()),
            attributes: None,
            bounding_box: Some(BoundingBox {
                x: 10.0,
                y: 20.0,
                width: 100.0,
                height: 40.0,
            }),
            visible: true,
            match_count: 1,
            match_index: 0,
        };
        let json = serde_json::to_string(&el).unwrap();
        let parsed: ElementInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.selector, "button.submit");
        assert_eq!(parsed.tag_name, "button");
        assert_eq!(parsed.text.as_deref(), Some("Submit"));
        assert!(parsed.visible);
    }

    #[test]
    fn test_element_info_omits_none_fields() {
        let el = ElementInfo {
            selector: "div".to_string(),
            tag_name: "div".to_string(),
            text: None,
            attributes: None,
            bounding_box: None,
            visible: false,
            match_count: 1,
            match_index: 0,
        };
        let json = serde_json::to_string(&el).unwrap();
        assert!(!json.contains("text"));
        assert!(!json.contains("attributes"));
        assert!(!json.contains("bounding_box"));
    }

    #[test]
    fn test_network_request_roundtrip() {
        let req = NetworkRequest {
            url: "https://example.com/api".to_string(),
            method: "GET".to_string(),
            status: Some(200),
            resource_type: Some("fetch".to_string()),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: NetworkRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.status, Some(200));
    }

    #[test]
    fn test_console_message_roundtrip() {
        let msg = ConsoleMessage {
            level: "error".to_string(),
            text: "Uncaught TypeError".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: ConsoleMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.level, "error");
    }

    #[test]
    fn test_bounding_box_roundtrip() {
        let bb = BoundingBox {
            x: 10.5,
            y: 20.0,
            width: 800.0,
            height: 600.0,
        };
        let json = serde_json::to_string(&bb).unwrap();
        let parsed: BoundingBox = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.x, 10.5);
        assert_eq!(parsed.width, 800.0);
    }
}
