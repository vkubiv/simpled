using System.Text.Json;
using System.Text.Json.Serialization;
using core.Infra;
using Stubble.Core.Builders;

namespace core.Services.Mailer;

public class MailContent
{
    public required string Subject { get; set; }
    public required string Body { get; set; }
}

public class MainTemplater
{
    public static async Task<MainTemplater> LoadFromDiskStorage(string langsDataPath,
        IReadOnlyCollection<string> languages)
    {
        var langsData = new Dictionary<string, LangData>();
        foreach (var language in languages)
        {
            var langTemplatesDirectory = $"{langsDataPath}/{language}/";
            var files = Directory.GetFiles(langTemplatesDirectory, "*");
            var emailTemplates = new Dictionary<string, MailTemplate>();
            foreach (var file in files)
            {
                var fileName = Path.GetFileNameWithoutExtension(file);
                var txtContent = await File.ReadAllTextAsync(file);
                // File is text file, first line is subject, rest is body
                var lines = txtContent.Split(Separator, StringSplitOptions.RemoveEmptyEntries);
                if (lines.Length < 2)
                {
                    throw new InvalidOperationException($"File {file} does not contain enough lines for template.");
                }
                var subject = lines[0];
                var body = string.Join("\n", lines, 1, lines.Length - 1);
                
                emailTemplates[fileName] = new MailTemplate
                {
                    Subject = subject,
                    Body = body
                };
            }
            
            langsData[language] = new LangData
            {
                EmailTemplates = emailTemplates
            };
        }
        
        return new MainTemplater(langsData);
    }

    public static async Task<MainTemplater> LoadFromExtHttpStorage(string externalStorageUri,
        IReadOnlyCollection<string> languages)
    {
        var langsData = new Dictionary<string, LangData>();
        foreach (var language in languages)
        {
            var jsonContent = await HttpInfra.ReadRemoteJsonToString($"{externalStorageUri}/{language}.json");

#pragma warning disable CA1869
            var options = new JsonSerializerOptions
            {
                PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
            };
#pragma warning restore CA1869

            langsData[language] =
                JsonSerializer.Deserialize<LangData>(jsonContent, options: options) ??
                throw new JsonException($"Fail to parse {externalStorageUri}/{language}.json");
        }

        return new MainTemplater(langsData);
    }

    public MainTemplater(Dictionary<string, LangData> langsData)
    {
        _langsData = langsData;
    }

    public MailContent RenderMailContent(string language, string templateTextId, Dictionary<string, object> data)
    {
        var stubble = new StubbleBuilder().Build();

        if (!_langsData.ContainsKey(language))
        {
            language = "en";
        }
        
        var langData = _langsData[language];
        var template = langData.EmailTemplates[templateTextId];

        return new MailContent
        {
            Subject = stubble.Render(template.Subject, data),
            Body = stubble.Render(template.Body, data)
        };
    }

    private readonly Dictionary<string, LangData> _langsData;
    private static readonly char[] Separator = new[] { '\n', '\r' };
}

public class LangData
{
    [JsonPropertyName("email_templates")]
    public required Dictionary<string, MailTemplate> EmailTemplates { get; init; }
}

public class MailTemplate
{
    public required string Subject { get; set; }
    public required string Body { get; set; }
}