import Foundation
import Tauri
import UIKit
import WebKit

private struct FileArgs: Decodable {
    let path: String
    let filename: String?
}

private struct SharedLinkArgs: Decodable { let id: String }

final class MobileFilesPlugin: Plugin, UIDocumentPickerDelegate {
    private var captures: [String: PageCapture] = [:]
    private var captureSession: PageCaptureSession?
    @objc public func renderPage(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(RenderPageArgs.self)
        DispatchQueue.main.async {
            guard self.captures.isEmpty, let host = self.manager.viewController?.view else { invoke.reject("The page browser is still stopping. Retry shortly."); return }
            let sessionId = args.request.session_id.flatMap { $0.isEmpty ? nil : $0 } ?? args.request.id
            if self.captureSession?.id != sessionId { self.captureSession = PageCaptureSession(id: sessionId) }
            let capture = PageCapture(args, session: self.captureSession, success: { invoke.resolve($0) }, failure: { invoke.reject($0) }) { [weak self] id in self?.captures.removeValue(forKey: id) }
            self.captures[args.request.id] = capture
            capture.start(in: host)
        }
    }
    @objc public func cancelRender(_ invoke: Invoke) throws {
        let id = try invoke.parseArgs(CancelRenderArgs.self).id
        DispatchQueue.main.async { self.captures[id]?.cancel(); invoke.resolve(["cancelled": true]) }
    }
    @objc public func renderStatus(_ invoke: Invoke) throws {
        let id = try invoke.parseArgs(CancelRenderArgs.self).id
        DispatchQueue.main.async { invoke.resolve(["progress": self.captures[id]?.progress.map { $0 as Any } ?? NSNull()]) }
    }
    @objc public func sharedLinks(_ invoke: Invoke) throws {
        do { invoke.resolve(["items": try SharedURLInbox.appGroup().list()]) }
        catch { invoke.reject(error.localizedDescription) }
    }

    @objc public func dismissSharedLink(_ invoke: Invoke) throws {
        do {
            let args = try invoke.parseArgs(SharedLinkArgs.self)
            try SharedURLInbox.appGroup().dismiss(id: args.id)
            invoke.resolve(["dismissed": true])
        } catch { invoke.reject(error.localizedDescription) }
    }
    private var pendingSave: Invoke?

    @objc public override func load(webview: WKWebView) {
        DispatchQueue.main.async {
            webview.scrollView.pinchGestureRecognizer?.isEnabled = false
            webview.scrollView.minimumZoomScale = 1
            webview.scrollView.maximumZoomScale = 1
        }
    }

    private func archiveURL(_ path: String) throws -> URL {
        let url = URL(fileURLWithPath: path).standardizedFileURL.resolvingSymlinksInPath()
        let container = URL(fileURLWithPath: NSHomeDirectory()).standardizedFileURL.resolvingSymlinksInPath()
        guard url.path.hasPrefix(container.path + "/"),
              url.pathExtension.lowercased() == "zim",
              FileManager.default.fileExists(atPath: url.path) else {
            throw NSError(domain: "ZimStudio", code: 1, userInfo: [NSLocalizedDescriptionKey: "Archive is outside the app's storage or no longer exists"])
        }
        return url
    }

    @objc public func save(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(FileArgs.self)
        let url = try archiveURL(args.path)
        DispatchQueue.main.async {
            guard self.pendingSave == nil,
                  let presenter = self.manager.viewController,
                  presenter.presentedViewController == nil else {
                invoke.reject("Close the current dialog before saving another archive")
                return
            }
            self.pendingSave = invoke
            let picker = UIDocumentPickerViewController(forExporting: [url], asCopy: true)
            picker.delegate = self
            picker.shouldShowFileExtensions = true
            picker.allowsMultipleSelection = false
            presenter.present(picker, animated: true)
        }
    }

    public func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
        guard let invoke = pendingSave else { return }
        pendingSave = nil
        guard let url = urls.first else { invoke.resolve(["saved": false]); return }
        invoke.resolve(["saved": true, "filename": url.lastPathComponent] as [String: Any])
    }

    public func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
        pendingSave?.resolve(["saved": false])
        pendingSave = nil
    }

    @objc public func open(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(FileArgs.self)
        let url = try archiveURL(args.path)
        DispatchQueue.main.async {
            guard let presenter = self.manager.viewController,
                  presenter.presentedViewController == nil else {
                invoke.reject("Close the current dialog before opening an archive")
                return
            }
            let sheet = UIActivityViewController(activityItems: [url], applicationActivities: nil)
            if presenter.traitCollection.userInterfaceIdiom == .pad, let view = presenter.view {
                sheet.modalPresentationStyle = .popover
                sheet.popoverPresentationController?.sourceView = view
                sheet.popoverPresentationController?.sourceRect = view.safeAreaLayoutGuide.layoutFrame
                sheet.popoverPresentationController?.permittedArrowDirections = []
            } else {
                sheet.modalPresentationStyle = .pageSheet
            }
            presenter.present(sheet, animated: true)
            invoke.resolve(["opened": true])
        }
    }
}

@_cdecl("init_plugin_mobile_files")
public func initPlugin() -> Plugin {
    MobileFilesPlugin()
}
