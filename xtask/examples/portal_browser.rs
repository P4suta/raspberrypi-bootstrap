use anyhow::{Context, Result, ensure};
use reqwest::{Method, Url, blocking::Client};
use serde_json::{Value, json};
use std::{
    env, fs,
    net::TcpListener,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const ELEMENT: &str = "element-6066-11e4-a52e-4f735466cecf";
const SERVICES: [(&str, u16); 3] = [
    ("Camera storage humidity", 12443),
    ("System metrics", 8443),
    ("Notifications", 10443),
];

struct Driver(Child);

impl Drop for Driver {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Browser {
    client: Client,
    session: String,
    _driver: Driver,
}

impl Browser {
    fn start() -> Result<Self> {
        let port = TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port();
        let driver = Driver(
            Command::new(env::var_os("CHROMEDRIVER").unwrap_or_else(|| "chromedriver".into()))
                .arg(format!("--port={port}"))
                .stdout(Stdio::null())
                .spawn()
                .context("start ChromeDriver for the portal check")?,
        );
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(15))
            .build()?;
        let origin = format!("http://127.0.0.1:{port}");
        let deadline = Instant::now() + Duration::from_secs(10);
        while client.get(format!("{origin}/status")).send().is_err() {
            ensure!(Instant::now() < deadline, "ChromeDriver did not start");
            thread::sleep(Duration::from_millis(50));
        }
        let capabilities = json!({"capabilities":{"alwaysMatch":{"browserName":"chrome",
            "goog:chromeOptions":{"args":["--headless","--no-sandbox","--disable-dev-shm-usage",
            "--no-proxy-server","--host-resolver-rules=MAP device.example.ts.net ~NOTFOUND"]}}}});
        let response: Value = client
            .post(format!("{origin}/session"))
            .json(&capabilities)
            .send()?
            .json()?;
        let id = response["value"]["sessionId"]
            .as_str()
            .with_context(|| format!("Chrome session: {response}"))?;
        Ok(Self {
            client,
            session: format!("{origin}/session/{id}"),
            _driver: driver,
        })
    }

    fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        let request = self
            .client
            .request(method, format!("{}{path}", self.session));
        let request = if let Some(body) = body {
            request.json(&body)
        } else {
            request
        };
        let response = request.send()?;
        let status = response.status();
        let mut result: Value = response.json()?;
        ensure!(status.is_success(), "{path}: {status} {result}");
        Ok(result["value"].take())
    }

    fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.request(Method::POST, path, Some(body))
    }

    fn get(&self, path: &str) -> Result<Value> {
        self.request(Method::GET, path, None)
    }

    fn cdp(&self, command: &str, params: Value) -> Result<Value> {
        self.post("/goog/cdp/execute", json!({"cmd":command,"params":params}))
    }

    fn open(&self, path: &Path) -> Result<()> {
        let url = Url::from_file_path(path)
            .map_err(|()| anyhow::anyhow!("expected an absolute fixture path"))?;
        self.post("/url", json!({"url":url.as_str()}))?;
        Ok(())
    }

    fn elements(&self, selector: &str) -> Result<Vec<String>> {
        let elements = self.post(
            "/elements",
            json!({"using":"css selector","value":selector}),
        )?;
        elements
            .as_array()
            .context("element list")?
            .iter()
            .map(|element| Ok(element[ELEMENT].as_str().context("element ID")?.to_owned()))
            .collect()
    }

    fn fits(&self) -> Result<bool> {
        let metrics = self.cdp("Page.getLayoutMetrics", json!({}))?;
        let content = metrics["cssContentSize"]["width"]
            .as_f64()
            .context("content width")?;
        let viewport = metrics["cssLayoutViewport"]["clientWidth"]
            .as_f64()
            .context("viewport width")?;
        Ok(content <= viewport)
    }

    fn monochrome(&self) -> Result<bool> {
        const PROPERTIES: [&str; 13] = [
            "color",
            "border-bottom-color",
            "outline-color",
            "text-decoration-color",
            "background-color",
            "background-image",
            "box-shadow",
            "text-shadow",
            "filter",
            "backdrop-filter",
            "mix-blend-mode",
            "background-blend-mode",
            "opacity",
        ];
        let snapshot = self.cdp(
            "DOMSnapshot.captureSnapshot",
            json!({"computedStyles":PROPERTIES}),
        )?;
        let palette = snapshot["strings"].as_array().context("snapshot strings")?;
        let rows = snapshot["documents"][0]["layout"]["styles"]
            .as_array()
            .context("computed styles")?;
        let mut checked = 0;
        for row in rows {
            let row = row.as_array().context("style row")?;
            if row.is_empty() {
                continue;
            }
            ensure!(row.len() == PROPERTIES.len(), "missing computed style");
            checked += 1;
            for (property, index) in PROPERTIES.iter().zip(row) {
                let value = index
                    .as_u64()
                    .and_then(|i| palette.get(i as usize))
                    .and_then(Value::as_str)
                    .context("style value")?;
                let allowed = match *property {
                    "color" | "border-bottom-color" | "outline-color" | "text-decoration-color" => {
                        matches!(value, "rgb(0, 0, 0)" | "rgb(255, 255, 255)")
                    }
                    "background-color" => matches!(
                        value,
                        "rgb(0, 0, 0)" | "rgb(255, 255, 255)" | "rgba(0, 0, 0, 0)"
                    ),
                    "mix-blend-mode" | "background-blend-mode" => value == "normal",
                    "opacity" => value == "1",
                    _ => value == "none",
                };
                if !allowed {
                    return Ok(false);
                }
            }
        }
        ensure!(checked > 0, "no rendered styles");
        Ok(true)
    }
}

fn check(directory: &Path, browser: &Browser) -> Result<()> {
    let index = directory.join("index.html");
    let original = fs::read_to_string(&index)?;
    let styles = original
        .split_once("<style>")
        .and_then(|(_, rest)| rest.split_once("</style>"))
        .context("portal styles")?
        .0;
    ensure!(
        !["transparent", "rgba(", "hsla("]
            .iter()
            .any(|value| styles.to_ascii_lowercase().contains(value)),
        "explicit translucent color"
    );
    browser.open(&index)?;
    let heading = browser.elements("main > h1")?;
    ensure!(
        heading.len() == 1
            && browser.get(&format!("/element/{}/text", heading[0]))? == "Pi services",
        "page heading"
    );
    let links = browser.elements("nav[aria-label='Services'] a")?;
    ensure!(links.len() == SERVICES.len(), "named routes");
    for (link, (name, port)) in links.iter().zip(SERVICES) {
        ensure!(
            browser.get(&format!("/element/{link}/computedlabel"))? == name,
            "accessible link name"
        );
        ensure!(
            browser.get(&format!("/element/{link}/attribute/href"))?
                == format!("https://device.example.ts.net:{port}/"),
            "HTTPS destination"
        );
    }
    for (width, height) in [(1280, 800), (768, 400), (320, 640)] {
        browser.cdp(
            "Emulation.setDeviceMetricsOverride",
            json!({"width":width,"height":height,"deviceScaleFactor":1,"mobile":false}),
        )?;
        ensure!(browser.fits()?, "horizontal overflow at {width}");
        for link in &links {
            let rect = browser.get(&format!("/element/{link}/rect"))?;
            for dimension in ["width", "height"] {
                ensure!(
                    rect[dimension].as_f64().context("target size")? >= 48.0,
                    "small link target"
                );
            }
        }
    }
    ensure!(browser.monochrome()?, "unexpected portal palette");
    browser.post("/actions", json!({"actions":[{"type":"pointer","id":"mouse",
        "actions":[{"type":"pointerMove","duration":0,"origin":{(ELEMENT):links[0]},"x":0,"y":0}]}]}))?;
    ensure!(browser.monochrome()?, "unexpected hover palette");

    let variant = directory.join("browser-variant.html");
    for rule in [
        "color:#555",
        "background:linear-gradient(black,white)",
        "opacity:.5",
        "box-shadow:0 1px black",
        "filter:blur(1px)",
        "mix-blend-mode:multiply",
    ] {
        fs::write(
            &variant,
            original.replace("</style>", &format!("a {{ {rule} }}</style>")),
        )?;
        browser.open(&variant)?;
        ensure!(!browser.monochrome()?, "missed style change: {rule}");
    }
    let enlarged = original.replace("</style>", ":root{font-size:200%} *{letter-spacing:.12em;word-spacing:.16em;line-height:1.5}</style>")
        .replace(SERVICES[0].0, &"x".repeat(180))
        .replace("<h1>Pi services", "<h1>Serviceportal");
    fs::write(&variant, enlarged)?;
    browser.open(&variant)?;
    ensure!(browser.fits()?, "enlarged link overflows");

    let destination = directory.join("empty.html");
    let destination_url =
        Url::from_file_path(&destination).map_err(|()| anyhow::anyhow!("destination path"))?;
    let mut local_links = original;
    for (_, port) in SERVICES {
        local_links = local_links.replace(
            &format!("https://device.example.ts.net:{port}/"),
            &format!("{destination_url}#{port}"),
        );
    }
    fs::write(&variant, local_links)?;
    browser.open(&variant)?;
    for key in ["\u{e004}", "\u{e007}"] {
        browser.post("/actions", json!({"actions":[{"type":"key","id":"keyboard","actions":[{"type":"keyDown","value":key},{"type":"keyUp","value":key}]}]}))?;
        if key == "\u{e004}" {
            let focused = browser.elements("nav a:first-child:focus-visible")?;
            ensure!(
                focused.len() == 1
                    && browser.get(&format!("/element/{}/css/outline-style", focused[0]))?
                        == "solid",
                "keyboard focus"
            );
        }
    }
    ensure!(
        browser.get("/url")? == format!("{destination_url}#12443"),
        "keyboard navigation"
    );
    browser.open(&variant)?;
    let links = browser.elements("nav a")?;
    browser.post(&format!("/element/{}/click", links[2]), json!({}))?;
    ensure!(
        browser.get("/url")? == format!("{destination_url}#10443"),
        "pointer navigation"
    );
    ensure!(browser.elements("a")?.is_empty(), "links on empty portal");
    let empty = browser.elements("main p")?;
    ensure!(
        empty.len() == 1
            && browser
                .get(&format!("/element/{}/text", empty[0]))?
                .as_str()
                .context("empty message")?
                .contains("Add a display name"),
        "empty state"
    );
    Ok(())
}

fn main() -> Result<()> {
    let directory = fs::canonicalize(env::args_os().nth(1).context("fixture directory")?)?;
    let browser = Browser::start()?;
    let result = check(&directory, &browser);
    browser.request(Method::DELETE, "", None)?;
    result
}
