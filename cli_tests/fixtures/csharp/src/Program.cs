namespace Fixtures.Greetings;

public interface IMessageWriter
{
    void Write(string message);
}

public enum GreetingStyle
{
    Formal,
    Casual,
}

public readonly record struct Greeting(string Text, GreetingStyle Style);

public class Greeter<TWriter> where TWriter : IMessageWriter, new()
{
    private readonly TWriter writer = new();

    public string Prefix { get; init; } = "Hello";

    public Greeting Greet(string name, GreetingStyle style = GreetingStyle.Formal)
    {
        var text = $"{Prefix}, {name}!";
        writer.Write(text);
        return new Greeting(text, style);
    }

    public record Result(Greeting Greeting, bool Delivered);
}
