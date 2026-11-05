import Foundation
import UIKit
import WebKit

struct RenderRequestArgs: Decodable {
    let id: String
    let session_id: String?
    let url: String
    let delay_ms: UInt64
    let timeout_secs: UInt64
    let max_html_bytes: UInt64
    let scopes: [String]
}
struct RenderPageArgs: Decodable { let request: RenderRequestArgs; let script: String }
struct CancelRenderArgs: Decodable { let id: String }

// Only one crawl's temporary browser cache is retained. It never uses Safari's data.
final class PageCaptureSession {
    let id: String
    let dataStore = WKWebsiteDataStore.nonPersistent()
    var mediaRules: WKContentRuleList?
    init(id: String) { self.id = id }
}

final class PageCapture: NSObject, WKNavigationDelegate, WKUIDelegate {
    private let args: RenderPageArgs
    private let session: PageCaptureSession
    private let success: ([String: Any]) -> Void
    private let failure: (String) -> Void
    private let complete: (String) -> Void
    private var view: WKWebView?
    private var timer: Timer?
    private var finished = false
    private var evaluating = false
    private(set) var progress: [String: Any]?

    init(_ args: RenderPageArgs, session: PageCaptureSession? = nil, success: @escaping ([String: Any]) -> Void, failure: @escaping (String) -> Void, complete: @escaping (String) -> Void) {
        self.args = args; self.session = session ?? PageCaptureSession(id: args.request.id)
        self.success = success; self.failure = failure; self.complete = complete
    }
    func start(in host: UIView) {
        guard let url = URL(string: args.request.url), allowed(url) else { fail("Invalid page capture URL"); return }
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = session.dataStore
        configuration.allowsInlineMediaPlayback = true
        configuration.mediaTypesRequiringUserActionForPlayback = .all
        configuration.preferences.javaScriptCanOpenWindowsAutomatically = false
        let data = try? JSONEncoder().encode(Options(args.request))
        guard let data = data, let options = String(data: data, encoding: .utf8) else { fail("Invalid capture options"); return }
        configuration.userContentController.addUserScript(WKUserScript(source: "window.__zimsmithRenderOptions=\(options);\n\(args.script)", injectionTime: .atDocumentStart, forMainFrameOnly: true))
        let browser = WKWebView(frame: host.bounds, configuration: configuration)
        browser.navigationDelegate = self; browser.uiDelegate = self
        browser.isUserInteractionEnabled = false; browser.accessibilityElementsHidden = true
        // Keep the renderer attached and laid out behind the application UI.
        browser.alpha = 0.01
        host.insertSubview(browser, at: 0)
        view = browser
        let rules = "[{\"trigger\":{\"url-filter\":\".*\",\"resource-type\":[\"media\"]},\"action\":{\"type\":\"block\"}}]"
        let load: (WKContentRuleList) -> Void = { [weak self] list in
            guard let self = self, !self.finished else { return }
            self.session.mediaRules = list
            browser.configuration.userContentController.add(list)
            browser.load(URLRequest(url: url, timeoutInterval: Double(self.args.request.timeout_secs)))
            let timer = Timer(timeInterval: 0.5, repeats: true) { [weak self] _ in self?.poll() }
            self.timer = timer
            // Scrolling the main app must not suspend page-capture polling.
            RunLoop.main.add(timer, forMode: .common)
        }
        if let list = session.mediaRules { load(list) }
        else {
            WKContentRuleListStore.default().compileContentRuleList(forIdentifier: "ZimsmithCaptureNoMedia", encodedContentRuleList: rules) { [weak self] list, error in
                guard let list = list else { self?.fail(error?.localizedDescription ?? "Could not disable browser media downloads"); return }
                load(list)
            }
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + Double(args.request.timeout_secs) + 5) { [weak self] in self?.fail("Browser rendering timed out. Increase its timeout or retry.") }
    }
    private struct Options: Encodable {
        let delay_ms: UInt64; let timeout_secs: UInt64; let max_html_bytes: UInt64
        init(_ args: RenderRequestArgs) { delay_ms = args.delay_ms; timeout_secs = args.timeout_secs; max_html_bytes = args.max_html_bytes }
    }
    private func allowed(_ candidate: URL) -> Bool {
        guard ["http", "https"].contains(candidate.scheme ?? ""), candidate.user == nil, candidate.password == nil else { return false }
        return args.request.scopes.contains { value in
            guard let root = URL(string: value), root.scheme == candidate.scheme, root.host == candidate.host,
                  (root.port ?? (root.scheme == "https" ? 443 : 80)) == (candidate.port ?? (candidate.scheme == "https" ? 443 : 80)) else { return false }
            let path = root.path.isEmpty ? "/" : root.path
            let prefix = path.hasSuffix("/") ? path : path + "/"
            return path == "/" || candidate.path == path || candidate.path.hasPrefix(prefix)
        }
    }
    private func poll() {
        guard !finished, !evaluating, let view = view else { return }
        evaluating = true
        view.evaluateJavaScript("(()=>{try{return window.__zimsmithCapture?.poll()||null}catch(e){return {error:String(e)}}})()") { [weak self] value, error in
            guard let self = self, !self.finished else { return }
            self.evaluating = false
            if let error = error { self.fail("Browser capture failed: \(error.localizedDescription)"); return }
            guard let result = value as? [String: Any] else { return }
            if let error = result["error"] as? String { self.fail(error); return }
            if let progress = result["progress"] as? [String: Any] { self.progress = progress; return }
            guard let html = result["html"] as? String else { return }
            guard html.utf8.count <= min(8 * 1024 * 1024, Int(self.args.request.max_html_bytes)) else { self.fail("Rendered HTML exceeds its capture limit"); return }
            self.succeed(result)
        }
    }
    func cancel() { fail("Page capture paused") }
    private func cleanup() {
        timer?.invalidate(); timer = nil
        view?.stopLoading(); view?.navigationDelegate = nil; view?.uiDelegate = nil
        view?.removeFromSuperview(); view = nil
        complete(args.request.id)
    }
    private func fail(_ message: String) {
        guard !finished else { return }; finished = true
        cleanup(); failure(message)
    }
    private func succeed(_ result: [String: Any]) {
        guard !finished else { return }; finished = true
        cleanup(); success(result)
    }
    func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction, decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        guard let url = navigationAction.request.url else { decisionHandler(.cancel); return }
        if url.absoluteString == "about:blank" { decisionHandler(.allow); return }
        if navigationAction.targetFrame?.isMainFrame != true { decisionHandler(.cancel); return }
        guard allowed(url) else { decisionHandler(.cancel); fail("JavaScript navigation left the crawl scope"); return }
        decisionHandler(.allow)
    }
    func webView(_ webView: WKWebView, decidePolicyFor navigationResponse: WKNavigationResponse, decisionHandler: @escaping (WKNavigationResponsePolicy) -> Void) {
        if navigationResponse.isForMainFrame, let response = navigationResponse.response as? HTTPURLResponse {
            if response.statusCode == 429 || response.statusCode == 503 {
                decisionHandler(.cancel)
                succeed(["url": response.url?.absoluteString ?? args.request.url, "html": "", "settled": false, "resource_count": 0, "known_transfer_bytes": 0, "warnings": [], "rate_limit": ["url": response.url?.absoluteString ?? args.request.url, "status": response.statusCode, "retry_after": response.value(forHTTPHeaderField: "Retry-After").map { $0 as Any } ?? NSNull()]])
                return
            }
            if response.statusCode >= 400 { decisionHandler(.cancel); fail("HTTP \(response.statusCode) while loading the rendered page"); return }
        }
        decisionHandler(.allow)
    }
    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) { if !finished { fail(error.localizedDescription) } }
    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) { if !finished { fail(error.localizedDescription) } }
    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) { fail("iOS stopped the page browser. Resume to retry from the saved checkpoint.") }
    func webView(_ webView: WKWebView, didReceive challenge: URLAuthenticationChallenge, completionHandler: @escaping (URLSession.AuthChallengeDisposition, URLCredential?) -> Void) {
        if challenge.protectionSpace.authenticationMethod == NSURLAuthenticationMethodServerTrust { completionHandler(.performDefaultHandling, nil) }
        else { completionHandler(.cancelAuthenticationChallenge, nil); fail("Authenticated browser captures are not supported") }
    }
    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration, for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? { return nil }
    @available(iOS 15.0, *)
    func webView(_ webView: WKWebView, requestMediaCapturePermissionFor origin: WKSecurityOrigin, initiatedByFrame frame: WKFrameInfo, type: WKMediaCaptureType, decisionHandler: @escaping (WKPermissionDecision) -> Void) { decisionHandler(.deny) }
}
