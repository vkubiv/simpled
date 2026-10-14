using System.Net;
using System.Net.Http.Headers;

namespace core.Infra;

public static class HttpInfra
{
    public static async Task<string> ReadRemoteJsonToString(string httpUrl)
    {
        using HttpClient client = new();
        client.DefaultRequestHeaders.Accept.Clear();
        client.DefaultRequestHeaders.Accept.Add(
            new MediaTypeWithQualityHeaderValue("application/json"));
        client.DefaultRequestHeaders.Add("User-Agent", ".NET Foundation Repository Reporter");

        await using Stream stream =
            await client.GetStreamAsync(new Uri(httpUrl));

        using var reader = new StreamReader(stream);
        return await reader.ReadToEndAsync();
    }
    
    public static async Task<bool> CheckFileExist(string httpUrl)
    {
        using var client = new HttpClient();
        using var request = new HttpRequestMessage(HttpMethod.Head, httpUrl);
            
        var response = await client.SendAsync(request);

        return response.StatusCode == HttpStatusCode.OK;
    }
}