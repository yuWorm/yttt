import SafariServices
import SwiftUI
import UIKit

/// The signed-in relay account: what the core returned, kept in the Keychain
/// on this device only. `account` goes back to the core to pair and connect.
struct RelayAccount: Codable, Equatable {
    var account: Data
    var email: String
    var controlURL: String

    init(_ signedIn: SignedIn) {
        account = signedIn.account
        email = signedIn.email
        controlURL = signedIn.controlUrl
    }

    /// The control plane's host, for a subtitle.
    var controlHost: String { URL(string: controlURL)?.host() ?? controlURL }
}

struct AccountStore {
    private let store = SecretStore(service: "com.yttt.yai", account: "relay-account")

    func load() -> RelayAccount? {
        guard let data = try? store.load() else { return nil }
        return try? JSONDecoder().decode(RelayAccount.self, from: data)
    }

    func save(_ account: RelayAccount) {
        guard let data = try? JSONEncoder().encode(account) else { return }
        try? store.save(data)
    }

    func delete() { store.delete() }
}

extension Error {
    /// What to show for an error from the core.
    var message: String {
        switch self as? MobileCoreError {
        case .Failed(let reason)?: reason
        case .HostTooOld?: "电脑上的 yttt 版本太旧"
        case .SignInExpired?: "Relay 账号的登录已过期"
        case .Cancelled?: "已取消"
        case nil: localizedDescription
        }
    }
}

/// The settings page's account section.
struct AccountSection: View {
    @EnvironmentObject private var model: AppModel
    @State private var details: AccountDetails?
    @State private var signingOut = false

    var body: some View {
        Section {
            if let account = model.account {
                HStack(spacing: 12) {
                    IconTile(systemName: "person.crop.circle", color: .orange)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(account.email)
                        Text(account.controlHost).font(.caption).foregroundStyle(.secondary)
                    }
                }
                if let details {
                    LabeledContent("套餐", value: details.plan)
                    LabeledContent("设备", value: "\(details.devices) / \(details.maxDevices)")
                    if details.suspended {
                        Text("账号已暂停，Relay 暂时不能用").foregroundStyle(.red)
                    }
                }
                Button("退出登录", role: .destructive) { signingOut = true }
            } else {
                NavigationLink {
                    SignInView()
                } label: {
                    HStack(spacing: 12) {
                        IconTile(systemName: "person.crop.circle.badge.plus", color: .orange)
                        Text("登录 Relay 账号")
                    }
                }
            }
        } header: {
            Text("Relay 账号")
        } footer: {
            if let notice = model.accountNotice {
                Text(notice).foregroundStyle(.red)
            } else if model.account == nil {
                Text("登录后手机经你的 Relay 连接电脑，不在同一个网络也能用。")
            } else if model.session != nil, model.relaySource != .account {
                Text("没能从账号拿到 Relay 凭证，这次只能直连。")
            } else {
                Text("手机经这个账号的 Relay 连接电脑。")
            }
        }
        .task(id: model.account) {
            details = nil
            guard let account = model.account else { return }
            do {
                details = try await model.core.accountDetails(account: account.account)
            } catch MobileCoreError.SignInExpired {
                await model.signOut(notice: "Relay 账号的登录已过期，请重新登录。")
            } catch {}
        }
        .confirmationDialog("退出 Relay 账号？", isPresented: $signingOut, titleVisibility: .visible) {
            Button("退出登录", role: .destructive) { Task { await model.signOut() } }
        } message: {
            Text("退出后，手机只能在同一个网络里直连电脑，或经自建的 Relay。")
        }
    }
}

/// The pairing page's first step: sign in, so pairing goes through the relay.
struct AccountStep: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        Section {
            if let account = model.account {
                HStack(spacing: 12) {
                    IconTile(systemName: "checkmark.seal", color: .green)
                    VStack(alignment: .leading, spacing: 2) {
                        Text("已登录 Relay 账号")
                        Text(account.email).font(.caption).foregroundStyle(.secondary)
                    }
                }
            } else {
                NavigationLink {
                    SignInView()
                } label: {
                    HStack(spacing: 12) {
                        IconTile(systemName: "person.crop.circle.badge.plus", color: .orange)
                        VStack(alignment: .leading, spacing: 2) {
                            Text("先登录 Relay 账号")
                            Text("登录后，手机在外面也能连上电脑。")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
            }
        } header: {
            Text("第 1 步 · 账号")
        } footer: {
            if model.account == nil {
                Text(model.accountNotice ?? "只在同一个网络里用，或用自建的 Relay，可以不登录。")
            }
        }
    }
}

/// Signing in: in the browser, where a password manager fills in the
/// console's page, or with the email and password here.
struct SignInView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @AppStorage("relayControlURL") private var controlURL = "https://"
    @State private var email = ""
    @State private var password = ""
    @State private var busy = false
    @State private var error: String?
    @State private var browser: BrowserPage?
    /// The browser sign-in under way; closing the browser cancels it.
    @State private var pending: BrowserSignIn?

    var body: some View {
        Form {
            Section {
                TextField("https://control.example.com", text: $controlURL)
                    .keyboardType(.URL)
                    .textContentType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
            } header: {
                Text("控制台地址")
            } footer: {
                Text("和电脑上“设置 ▸ 远程访问 ▸ Relay 账号”里的地址相同。")
            }
            Section {
                Button { Task { await signInInBrowser() } } label: {
                    HStack(spacing: 12) {
                        IconTile(systemName: "safari", color: .blue)
                        Text("在浏览器中登录")
                        Spacer()
                        if busy, browser != nil { ProgressView() }
                    }
                }
                .disabled(busy || !validURL)
            } footer: {
                Text("在网页上登录并确认，可以用钥匙串或密码管理器自动填充。")
            }
            Section("用邮箱和密码") {
                TextField("邮箱", text: $email)
                    .keyboardType(.emailAddress)
                    .textContentType(.username)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                SecureField("密码", text: $password)
                    .textContentType(.password)
                    .onSubmit { Task { await signInWithPassword() } }
                Button { Task { await signInWithPassword() } } label: {
                    HStack {
                        Text("登录")
                        Spacer()
                        if busy, browser == nil { ProgressView() }
                    }
                }
                .disabled(busy || !validURL || email.isEmpty || password.isEmpty)
            }
            if let error {
                Section { Text(error).foregroundStyle(.red) }
            }
        }
        .navigationTitle("登录 Relay 账号")
        .navigationBarTitleDisplayMode(.inline)
        .sheet(item: $browser, onDismiss: { pending?.cancel() }) { page in
            SafariView(url: page.url) { pending?.cancel() }.ignoresSafeArea()
        }
    }

    private var trimmedURL: String { controlURL.trimmingCharacters(in: .whitespacesAndNewlines) }

    private var validURL: Bool {
        guard let url = URL(string: trimmedURL), let scheme = url.scheme, url.host() != nil else { return false }
        return scheme == "https" || scheme == "http"
    }

    private func signInInBrowser() async {
        busy = true
        error = nil
        defer { busy = false }
        do {
            let signIn = try await model.core.beginBrowserSignIn(controlUrl: trimmedURL, label: UIDevice.current.name)
            guard let url = URL(string: signIn.authorizeUrl()) else { return }
            pending = signIn
            browser = BrowserPage(url: url)
            let result = await Result(catching: { try await signIn.finish() })
            // Close the browser before leaving this page.
            pending = nil
            browser = nil
            await done(try result.get())
        } catch MobileCoreError.Cancelled {
        } catch {
            self.error = "登录失败：\(error.message)"
        }
    }

    private func signInWithPassword() async {
        guard !busy, validURL, !email.isEmpty, !password.isEmpty else { return }
        busy = true
        error = nil
        defer { busy = false }
        do {
            let signedIn = try await model.core.signInPassword(controlUrl: trimmedURL, email: email, password: password)
            await done(signedIn)
        } catch {
            self.error = "登录失败：\(error.message)"
        }
    }

    private func done(_ signedIn: SignedIn) async {
        password = ""
        controlURL = signedIn.controlUrl
        await model.signIn(signedIn)
        dismiss()
    }
}

extension Result where Failure == Error {
    init(catching body: () async throws -> Success) async {
        do { self = .success(try await body()) } catch { self = .failure(error) }
    }
}

/// The console's page, open in the browser until it sends the browser back.
struct BrowserPage: Identifiable {
    let id = UUID()
    let url: URL
}

/// Safari inside the app: the user's saved passwords and password managers
/// fill in the page, and its cookies stay away from the app.
struct SafariView: UIViewControllerRepresentable {
    let url: URL
    /// The user closed it with its own button.
    let onClose: () -> Void

    func makeUIViewController(context: Context) -> SFSafariViewController {
        let controller = SFSafariViewController(url: url)
        controller.dismissButtonStyle = .cancel
        controller.delegate = context.coordinator
        return controller
    }

    func updateUIViewController(_ controller: SFSafariViewController, context: Context) {
        context.coordinator.onClose = onClose
    }

    func makeCoordinator() -> Coordinator { Coordinator(onClose: onClose) }

    /// The sheet's `onDismiss` does not hear the browser closing itself.
    final class Coordinator: NSObject, SFSafariViewControllerDelegate {
        var onClose: () -> Void
        init(onClose: @escaping () -> Void) { self.onClose = onClose }
        func safariViewControllerDidFinish(_ controller: SFSafariViewController) { onClose() }
    }
}
