import Foundation
import Tauri
import UIKit
import UniformTypeIdentifiers
import WebKit

private struct FileArgs: Decodable {
    let path: String
    let filename: String?
}

private struct SharedLinkArgs: Decodable { let id: String }

final class MobileFilesPlugin: Plugin, UIDocumentPickerDelegate, UIDocumentInteractionControllerDelegate {
    private var captures: [String: PageCapture] = [:]
    @objc public func renderPage(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(RenderPageArgs.self)
        DispatchQueue.main.async {
            guard self.captures.isEmpty, let host = self.manager.viewController?.view else { invoke.reject("The page browser is still stopping. Retry shortly."); return }
            let capture = PageCapture(args, success: { invoke.resolve($0) }, failure: { invoke.reject($0) }) { [weak self] id in self?.captures.removeValue(forKey: id) }
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
    private var documentController: UIDocumentInteractionController?

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
            let document = UIDocumentInteractionController(url: url)
            document.name = url.lastPathComponent
            document.uti = UTType(filenameExtension: "zim")?.identifier ?? "org.openzim.zim"
            document.delegate = self
            self.documentController = document
            let view = presenter.view!
            let anchor = CGRect(x: view.bounds.midX, y: view.bounds.midY, width: 1, height: 1)
            if document.presentOpenInMenu(from: anchor, in: view, animated: true) {
                invoke.resolve(["opened": true])
            } else {
                self.documentController = nil
                invoke.reject("No ZIM viewer is installed. Install a compatible viewer, then tap Open ZIM again.")
            }
        }
    }
}

@_cdecl("init_plugin_mobile_files")
public func initPlugin() -> Plugin {
    MobileFilesPlugin()
}
