import UIKit
import UniformTypeIdentifiers

final class ShareViewController: UIViewController {
    private let message = UILabel()
    private let address = UILabel()
    private let add = UIButton(type: .system)
    private var sharedURL: URL?
    private var pageTitle = ""

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground
        preferredContentSize = CGSize(width: 360, height: 300)
        let heading = UILabel()
        heading.text = "Add to Zimsmith"
        heading.font = .preferredFont(forTextStyle: .title2)
        heading.adjustsFontForContentSizeCategory = true
        address.font = .preferredFont(forTextStyle: .body)
        address.numberOfLines = 3
        address.lineBreakMode = .byTruncatingMiddle
        message.font = .preferredFont(forTextStyle: .body)
        message.numberOfLines = 0
        message.text = "Save this link, then open Zimsmith to review and start the crawl."
        add.setTitle("Save link", for: .normal)
        add.titleLabel?.font = .preferredFont(forTextStyle: .headline)
        add.isEnabled = false
        add.addTarget(self, action: #selector(saveLink), for: .touchUpInside)
        let cancel = UIButton(type: .system)
        cancel.setTitle("Cancel", for: .normal)
        cancel.addTarget(self, action: #selector(cancelShare), for: .touchUpInside)
        let stack = UIStackView(arrangedSubviews: [heading, address, message, add, cancel])
        stack.axis = .vertical
        stack.spacing = 16
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 24),
            stack.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -24),
            stack.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 24),
            stack.bottomAnchor.constraint(lessThanOrEqualTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -24),
            add.heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
            cancel.heightAnchor.constraint(greaterThanOrEqualToConstant: 44)
        ])
        let items = extensionContext?.inputItems.compactMap { $0 as? NSExtensionItem } ?? []
        pageTitle = String((items.first?.attributedTitle?.string ?? "").prefix(180))
        let providers = items.flatMap { $0.attachments ?? [] }
        load(providers, at: 0, type: UTType.propertyList.identifier) {
            self.load(providers, at: 0, type: UTType.url.identifier) {
                self.load(providers, at: 0, type: UTType.plainText.identifier) {
                    self.message.text = "No website link was found. Share a URL from your browser."
                }
            }
        }
    }

    private func load(_ providers: [NSItemProvider], at index: Int, type: String, exhausted: @escaping () -> Void) {
        guard index < providers.count else { exhausted(); return }
        let provider = providers[index]
        guard provider.hasItemConformingToTypeIdentifier(type) else { load(providers, at: index + 1, type: type, exhausted: exhausted); return }
        provider.loadItem(forTypeIdentifier: type, options: nil) { item, _ in
            let page = (item as? [String: Any])?[NSExtensionJavaScriptPreprocessingResultsKey] as? [String: Any]
            let text = (page?["url"] as? String) ?? (item as? URL)?.absoluteString ?? (item as? String) ?? (item as? Data).flatMap { String(data: $0, encoding: .utf8) } ?? ""
            DispatchQueue.main.async {
                if let title = page?["title"] as? String { self.pageTitle = String(title.prefix(180)) }
                let detector = try? NSDataDetector(types: NSTextCheckingResult.CheckingType.link.rawValue)
                let matches = text.utf8.count <= 32_768 ? detector?.matches(in: text, range: NSRange(text.startIndex..., in: text)) ?? [] : []
                let urls = matches.compactMap { $0.url }.filter { ["http", "https"].contains($0.scheme?.lowercased() ?? "") }
                if urls.count == 1, let url = urls.first, url.user == nil, url.password == nil, url.host != nil {
                    self.sharedURL = url
                    self.address.text = url.absoluteString
                    self.add.isEnabled = true
                } else {
                    self.load(providers, at: index + 1, type: type, exhausted: exhausted)
                }
            }
        }
    }

    @objc private func saveLink() {
        guard let url = sharedURL else { return }
        do {
            try SharedURLInbox.appGroup().save(text: url.absoluteString, title: pageTitle)
            extensionContext?.completeRequest(returningItems: nil)
        } catch {
            message.text = error.localizedDescription
        }
    }

    @objc private func cancelShare() {
        extensionContext?.cancelRequest(withError: NSError(domain: NSCocoaErrorDomain, code: NSUserCancelledError))
    }
}
