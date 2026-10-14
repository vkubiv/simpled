using System.Security.Cryptography;
using System.Text;

namespace core.Services.GitHub;

/// <summary>
/// AES-GCM over the GitHub tokens organizations save, so a database dump alone does not hand out repository
/// access. The key is the SHA-256 of the github_token_key secret: any secret string works, and rotating it
/// means asking every organization for its tokens again.
/// </summary>
public sealed class TokenCipher
{
    private const int NonceSize = 12;
    private const int TagSize = 16;

    private readonly byte[] _key;

    public TokenCipher(string secret)
    {
        _key = SHA256.HashData(Encoding.UTF8.GetBytes(secret));
    }

    public string Encrypt(string token)
    {
        var plain = Encoding.UTF8.GetBytes(token);
        var output = new byte[NonceSize + TagSize + plain.Length];
        var nonce = output.AsSpan(0, NonceSize);
        RandomNumberGenerator.Fill(nonce);
        using var aes = new AesGcm(_key, TagSize);
        aes.Encrypt(nonce, plain, output.AsSpan(NonceSize + TagSize), output.AsSpan(NonceSize, TagSize));
        return Convert.ToBase64String(output);
    }

    public string Decrypt(string ciphertext)
    {
        var input = Convert.FromBase64String(ciphertext);
        var plain = new byte[input.Length - NonceSize - TagSize];
        using var aes = new AesGcm(_key, TagSize);
        aes.Decrypt(input.AsSpan(0, NonceSize), input.AsSpan(NonceSize + TagSize), input.AsSpan(NonceSize, TagSize),
            plain);
        return Encoding.UTF8.GetString(plain);
    }
}
