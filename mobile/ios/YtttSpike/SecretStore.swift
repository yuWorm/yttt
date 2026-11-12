import Foundation
import Security

struct KeychainError: Error, CustomStringConvertible {
    let status: OSStatus
    var description: String { "Keychain error \(status)" }
}

/// The device secret in the Keychain: readable in the background once the
/// phone has been unlocked after boot, kept on this device only (not synced
/// to iCloud, not restored onto another phone).
struct SecretStore {
    let service: String
    let account: String
    /// Where the spike kept it in plain text; moved in once, then deleted.
    let legacyFile: URL?

    private var query: [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: service,
         kSecAttrAccount as String: account]
    }

    func load() throws -> Data? {
        var query = self.query
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess else { throw KeychainError(status: status) }
        return item as? Data
    }

    func save(_ secret: Data) throws {
        SecItemDelete(query as CFDictionary)
        var item = query
        item[kSecValueData as String] = secret
        item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        let status = SecItemAdd(item as CFDictionary, nil)
        guard status == errSecSuccess else { throw KeychainError(status: status) }
    }

    func delete() {
        SecItemDelete(query as CFDictionary)
    }

    /// The secret: from the Keychain, else moved from the old plain file,
    /// else made now.
    func deviceSecret() throws -> Data {
        if let secret = try load(), secret.count == 32 { return secret }
        if let legacyFile, let old = try? Data(contentsOf: legacyFile), old.count == 32 {
            try save(old)
            // Only once the Keychain has it.
            if try load() == old { try? FileManager.default.removeItem(at: legacyFile) }
            return old
        }
        var bytes = [UInt8](repeating: 0, count: 32)
        guard SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes) == errSecSuccess else {
            throw KeychainError(status: errSecAllocate)
        }
        let secret = Data(bytes)
        try save(secret)
        return secret
    }
}
